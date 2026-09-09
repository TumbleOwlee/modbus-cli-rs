//! `#[derive(Focus)]` and the `#[focusable]` attribute.

use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{
    Expr, Field, Fields, Ident, Meta, MetaNameValue, Token, Type, Visibility, parse::Parser,
    punctuated::Punctuated,
};

/// One focusable field, as gathered from a `#[focus]`/`#[focus(when = …)]`/`#[focus(nested)]`
/// attribute.
struct Definition {
    widget_name: Ident,
    enum_field: Ident,
    when: Option<Expr>,
    nested: bool,
}

/// Collect the `#[focus]`-tagged fields of a struct in declaration order.
fn collect_definitions(fields: &Fields) -> syn::Result<Vec<Definition>> {
    let mut definitions = vec![];

    for field in fields.iter() {
        let mut found = false;
        let mut when: Option<Expr> = None;
        let mut nested = false;

        for attr in field.attrs.iter() {
            if !attr.path().is_ident("focus") {
                continue;
            }
            found = true;

            // No arguments, just `#[focus]`.
            if let Meta::Path(_) = attr.meta {
                continue;
            }

            // Parse arguments for `#[focus(when = some_condition)]`.
            let args = attr
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .map_err(|_| {
                    syn::Error::new_spanned(
                        attr,
                        "Invalid syntax for #[focus] attribute, expected #[focus(when = some_condition)]",
                    )
                })?;
            for arg in args {
                match arg {
                    Meta::NameValue(MetaNameValue { path, value, .. }) if path.is_ident("when") => {
                        when = Some(value);
                    }
                    Meta::Path(p) if p.is_ident("nested") => {
                        nested = true;
                    }
                    other => {
                        return Err(syn::Error::new_spanned(
                            &other,
                            "Invalid argument for #[focus] attribute, expected key-value pairs like #[focus(when = some_condition)]",
                        ));
                    }
                }
            }
        }

        if found {
            let ident = field.ident.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(field, "FocusSwitch only works on named fields with ident.")
            })?;
            let enum_field = ident.to_string().to_case(Case::Pascal);
            definitions.push(Definition {
                widget_name: ident.clone(),
                enum_field: Ident::new(&enum_field, Span::call_site()),
                when,
                nested,
            });
        }
    }

    Ok(definitions)
}

/// Derives focus cycling, whole-view `SetFocus`/`IsFocus`, and event dispatch.
pub fn expand_focus(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let identifier = &input.ident;
    let (impl_generic, ty_generic, where_clause) = &input.generics.split_for_impl();

    let s = match &input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return Err(syn::Error::new_spanned(
                identifier,
                "Focus can only be derived for structs",
            ));
        }
    };

    let definitions = collect_definitions(&s.fields)?;

    // Set by `expand_focusable` (via the `#[focus_nestable]` marker attribute) when the struct
    // opted in with `#[focusable(nestable)]`. Gates generation of `NestedFocus` and its
    // supporting methods so every other `#[derive(Focus)]` struct's generated code is unaffected.
    let is_nestable = input
        .attrs
        .iter()
        .any(|a| a.path().is_ident("focus_nestable"));

    if definitions.is_empty() {
        return Err(syn::Error::new_spanned(
            identifier,
            "Focus derive requires at least one #[focus] field",
        ));
    }

    let def_len = definitions.len();

    let enum_name = Ident::new(&format!("{identifier}Focus"), Span::call_site());

    let enum_fields = definitions.iter().map(|i| &i.enum_field);
    let impl_array = quote! {
        let focuses = [#(#enum_name::#enum_fields),*];
    };

    let mut impl_disable = quote! {};
    for def in definitions.iter() {
        let name = &def.widget_name;
        let enum_field = &def.enum_field;
        impl_disable.extend(quote! {
            #enum_name::#enum_field => {ferrowl_ui::traits::SetFocus::set_focused(&mut self.#name, false);}
        });
    }
    let impl_disable = quote! {
        match self.focus {
            #impl_disable
            _ => {unreachable!("Invalid focus state");},
        }
    };

    // Built once per direction: for a plain field the two
    // are byte-for-byte identical (the `else` branch below); for a `#[focus(nested)]` field,
    // forward entry calls the direction-aware
    // "enter at first eligible" helper and backward entry calls "enter at last eligible" instead
    // of the direction-blind `SetFocus::set_focused(true)` — and, since finding no eligible inner
    // pane is possible (a nested field can be structurally `when`-eligible yet have zero eligible
    // panes of its own), a failed entry does not `break`, letting the surrounding scan continue to
    // the next candidate exactly as it already does for an ordinary ineligible field.
    let impl_enable_dir = |forward: bool| {
        let mut arms = quote! {};
        for def in definitions.iter() {
            let name = &def.widget_name;
            let enum_field = &def.enum_field;
            let when = if let Some(when) = &def.when {
                quote! {
                    && #when
                }
            } else {
                quote! {}
            };

            let enter = if def.nested {
                let entry_call = if forward {
                    quote! { self.#name.__focus_enter_first_eligible() }
                } else {
                    quote! { self.#name.__focus_enter_last_eligible() }
                };
                quote! {
                    if #entry_call {
                        self.focus = #enum_name::#enum_field;
                        break;
                    }
                }
            } else {
                quote! {
                    ferrowl_ui::traits::SetFocus::set_focused(&mut self.#name, true);
                    self.focus = #enum_name::#enum_field;
                    break;
                }
            };

            arms.extend(quote! {
                if current_focus == #enum_name::#enum_field #when {
                    #enter
                }
            });
        }
        arms
    };
    let impl_enable_forward = impl_enable_dir(true);
    let impl_enable_backward = impl_enable_dir(false);

    // Forward and reverse traversal differ by the per-step `delta` (forward = +1, reverse =
    // +(len-1), i.e. -1 mod len) and by which direction's `impl_enable_*` is spliced in.
    let focus_loop = |delta: TokenStream, enable: &TokenStream| {
        quote! {
            #impl_array

            #impl_disable

            let index = focuses.iter().position(|f| *f == self.focus).unwrap();

            let mut current_index = (index + #delta) % #def_len;

            loop {
                let current_focus = focuses[current_index];

                #enable

                if current_index == index {
                    break;
                }

                current_index = (current_index + #delta) % #def_len;
            }
        }
    };
    let impl_previous = focus_loop(quote! { (#def_len - 1) }, &impl_enable_backward);
    let impl_next = focus_loop(quote! { 1 }, &impl_enable_forward);

    let focus_def = quote! {
        impl #impl_generic #identifier #ty_generic #where_clause {
            // `% #def_len` collapses to `% 1` for single-field views; that is
            // correct (it always yields the one field) but trips `modulo_one`.
            #[allow(clippy::modulo_one)]
            pub fn focus_previous(&mut self) {
                #impl_previous
            }
            #[allow(clippy::modulo_one)]
            pub fn focus_next(&mut self) {
                #impl_next
            }
        }
    };

    // `set_focused`/`is_focused` for the whole view (a `#[focus]`-bearing struct is itself a
    // focusable node, so it composes with parent views). Enabling restores the remembered
    // pane if its `#[focus(when=…)]` guard still holds, else the first eligible pane;
    // disabling unfocuses every child and keeps the remembered pane.
    let mut impl_clear_all = quote! {};
    let mut impl_eligibility = quote! {};
    let mut impl_candidates = quote! {};
    let mut impl_focus_one = quote! {};
    for def in definitions.iter() {
        let name = &def.widget_name;
        let enum_field = &def.enum_field;
        let when = match &def.when {
            Some(when) => quote! { #when },
            None => quote! { true },
        };
        impl_clear_all.extend(quote! {
            ferrowl_ui::traits::SetFocus::set_focused(&mut self.#name, false);
        });
        impl_eligibility.extend(quote! {
            #enum_name::#enum_field => #when,
        });
        impl_candidates.extend(quote! {
            (#enum_name::#enum_field, #when),
        });
        impl_focus_one.extend(quote! {
            #enum_name::#enum_field => ferrowl_ui::traits::SetFocus::set_focused(&mut self.#name, true),
        });
    }
    let set_focus_def = quote! {
        impl #impl_generic ferrowl_ui::traits::IsFocus for #identifier #ty_generic #where_clause {
            fn is_focused(&self) -> bool {
                self.view_focused
            }
        }
        impl #impl_generic ferrowl_ui::traits::SetFocus for #identifier #ty_generic #where_clause {
            fn set_focused(&mut self, focus: bool) {
                self.view_focused = focus;
                #impl_clear_all
                if !focus {
                    return;
                }
                let remembered_ok = match self.focus {
                    #impl_eligibility
                };
                if !remembered_ok {
                    let candidates = [ #impl_candidates ];
                    if let Some(&(f, _)) = candidates.iter().find(|&&(_, ok)| ok) {
                        self.focus = f;
                    }
                }
                match self.focus {
                    #impl_focus_one
                }
            }
        }
    };

    let enum_fields = definitions.iter().map(|i| &i.enum_field);
    let enum_def = quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum #enum_name {
            #(#enum_fields),*
        }
    };

    // A `#[focus(nested)]` field's arm additionally tries
    // `NestedFocus` stepping on an `Unhandled` Tab/BackTab from that field's own `handle_events`,
    // converting to `Consumed` on success or re-emitting the original `Unhandled` on failure (so
    // it bubbles to whichever outer call site owns this struct's own `focus_next`/`focus_previous`
    // fallback). Every non-nested field's arm forwards directly to that field's own `handle_events`.
    let mut impl_handle_events = quote! {};
    for def in definitions.iter() {
        let from = &def.widget_name;
        let from_enum = &def.enum_field;
        let arm = if def.nested {
            quote! {
                match ferrowl_ui::traits::HandleEvents::handle_events(&mut self.#from, modifiers, code) {
                    ferrowl_ui::EventResult::Unhandled(m, crossterm::event::KeyCode::Tab) => {
                        if ferrowl_ui::traits::NestedFocus::try_focus_next(&mut self.#from) {
                            ferrowl_ui::EventResult::Consumed
                        } else {
                            ferrowl_ui::EventResult::Unhandled(m, crossterm::event::KeyCode::Tab)
                        }
                    }
                    ferrowl_ui::EventResult::Unhandled(m, crossterm::event::KeyCode::BackTab) => {
                        if ferrowl_ui::traits::NestedFocus::try_focus_previous(&mut self.#from) {
                            ferrowl_ui::EventResult::Consumed
                        } else {
                            ferrowl_ui::EventResult::Unhandled(m, crossterm::event::KeyCode::BackTab)
                        }
                    }
                    other => other,
                }
            }
        } else {
            quote! { ferrowl_ui::traits::HandleEvents::handle_events(&mut self.#from, modifiers, code) }
        };
        impl_handle_events.extend(quote! {
            #enum_name::#from_enum => #arm,
        });
    }

    let handle_def = quote! {
        impl #impl_generic ferrowl_ui::traits::HandleEvents for #identifier #ty_generic #where_clause {
            fn handle_events(&mut self, modifiers: crossterm::event::KeyModifiers, code: crossterm::event::KeyCode) -> ferrowl_ui::EventResult {
                match self.focus {
                    #impl_handle_events
                    _ => unreachable!("Invalid focus state"),
                }
            }
        }
    };

    // `NestedFocus` support: a genuinely new, bounded (non-wrapping) scan, generated only for a
    // struct that opted in via `#[focusable(nestable)]`. Every other struct gets none of this.
    //
    // One shared per-field arm, parameterized by `extra` (the token stream run just before
    // committing to a candidate): the two scanning methods (`try_focus_next`/`_previous`) pass
    // `impl_disable` so the currently-focused field is disabled only once a next eligible target
    // is actually found (never eagerly); the two entry methods
    // (`__focus_enter_first_eligible`/`_last_eligible`) pass `self.view_focused = true;` instead,
    // since nothing is enabled yet on first entry (the struct was left fully disabled by its own
    // `SetFocus::set_focused(false)` — see `impl_clear_all` above — when it was last exited).
    let step_arm = |extra: &TokenStream| {
        let mut arms = quote! {};
        for def in definitions.iter() {
            let name = &def.widget_name;
            let enum_field = &def.enum_field;
            let when = if let Some(when) = &def.when {
                quote! { && #when }
            } else {
                quote! {}
            };
            arms.extend(quote! {
                if candidate == #enum_name::#enum_field #when {
                    #extra
                    self.focus = #enum_name::#enum_field;
                    ferrowl_ui::traits::SetFocus::set_focused(&mut self.#name, true);
                    return true;
                }
            });
        }
        arms
    };
    let step_arm_scan = step_arm(&impl_disable);
    let step_arm_enter = step_arm(&quote! { self.view_focused = true; });

    let nested_methods = if is_nestable {
        quote! {
            impl #impl_generic #identifier #ty_generic #where_clause {
                #[doc(hidden)]
                pub fn try_focus_next(&mut self) -> bool {
                    #impl_array
                    let index = focuses.iter().position(|f| *f == self.focus).unwrap();
                    for i in (index + 1)..#def_len {
                        let candidate = focuses[i];
                        #step_arm_scan
                    }
                    false
                }
                #[doc(hidden)]
                pub fn try_focus_previous(&mut self) -> bool {
                    #impl_array
                    let index = focuses.iter().position(|f| *f == self.focus).unwrap();
                    for i in (0..index).rev() {
                        let candidate = focuses[i];
                        #step_arm_scan
                    }
                    false
                }
                #[doc(hidden)]
                pub fn __focus_enter_first_eligible(&mut self) -> bool {
                    #impl_array
                    for i in 0..#def_len {
                        let candidate = focuses[i];
                        #step_arm_enter
                    }
                    false
                }
                #[doc(hidden)]
                pub fn __focus_enter_last_eligible(&mut self) -> bool {
                    #impl_array
                    for i in (0..#def_len).rev() {
                        let candidate = focuses[i];
                        #step_arm_enter
                    }
                    false
                }
            }
            impl #impl_generic ferrowl_ui::traits::NestedFocus for #identifier #ty_generic #where_clause {
                fn try_focus_next(&mut self) -> bool {
                    self.try_focus_next()
                }
                fn try_focus_previous(&mut self) -> bool {
                    self.try_focus_previous()
                }
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        #enum_def
        #focus_def
        #set_focus_def
        #handle_def
        #nested_methods
    })
}

/// Appends the `focus`/`view_focused` fields the `Focus` derive needs. `attr` is the raw
/// `#[focusable(...)]` argument list, e.g. `nestable` for `#[focusable(nestable)]` (empty for a
/// bare `#[focusable]`).
pub fn expand_focusable(
    attr: TokenStream,
    mut input: syn::DeriveInput,
) -> syn::Result<TokenStream> {
    let mut nestable = false;
    if !attr.is_empty() {
        let args = Punctuated::<Meta, Token![,]>::parse_terminated
            .parse2(attr)
            .map_err(|_| {
                syn::Error::new(
                    Span::call_site(),
                    "Invalid syntax for #[focusable] attribute, expected #[focusable(nestable)]",
                )
            })?;
        for arg in args {
            match arg {
                Meta::Path(p) if p.is_ident("nestable") => {
                    nestable = true;
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        &other,
                        "Invalid argument for #[focusable] attribute, expected #[focusable(nestable)]",
                    ));
                }
            }
        }
    }

    // Structs that also `#[derive(Builder)]` get `#[builder(default)]` on the injected
    // `view_focused` flag so callers needn't set it (it defaults to `false`); the `focus` field is
    // still set explicitly by those builders (its enum has no `Default`).
    let uses_builder = input.attrs.iter().any(|attr| {
        attr.path().is_ident("derive")
            && attr
                .parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
                .is_ok_and(|paths| paths.iter().any(|p| p.is_ident("Builder")))
    });

    let s = match &mut input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "#[focusable] can only be applied to structs",
            ));
        }
    };

    let named = match &mut s.fields {
        Fields::Named(named) => named,
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "#[focusable] only works on structs with named fields.",
            ));
        }
    };

    let focus_ty = input.ident.to_string().to_case(Case::Pascal) + "Focus";
    let focus_field = Field {
        attrs: Vec::new(),
        modifiers: Default::default(),
        vis: Visibility::Inherited,
        ident: Some(Ident::new("focus", Span::call_site())),
        colon_token: Some(Default::default()),
        ty: syn::parse_str::<Type>(&focus_ty)?,
        default: None,
    };
    let view_focused_attrs = if uses_builder {
        vec![syn::parse_quote!(#[builder(default)])]
    } else {
        Vec::new()
    };
    let view_focused_field = Field {
        attrs: view_focused_attrs,
        modifiers: Default::default(),
        vis: Visibility::Inherited,
        ident: Some(Ident::new("view_focused", Span::call_site())),
        colon_token: Some(Default::default()),
        ty: syn::parse_str::<Type>("bool")?,
        default: None,
    };

    named.named.push(focus_field);
    named.named.push(view_focused_field);

    if nestable {
        // Marker read by `expand_focus` (via `derive_focus`'s `attributes(focus,
        // focus_nestable)` registration) to gate generation of the `NestedFocus` impl and its
        // supporting methods onto only a struct that opted in.
        input.attrs.push(syn::parse_quote!(#[focus_nestable]));
    }

    Ok(quote! { #input })
}

#[cfg(test)]
mod tests {
    use super::{expand_focus, expand_focusable};
    use proc_macro2::TokenStream;

    #[test]
    fn rejects_struct_with_no_focus_fields() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct EmptyView {
                focus: EmptyViewFocus,
                view_focused: bool,
            }
        };

        let err = expand_focus(input).expect_err("expected zero-field struct to be rejected");
        assert_eq!(
            err.to_string(),
            "Focus derive requires at least one #[focus] field"
        );
    }

    #[test]
    fn rejects_invalid_focus_attribute_syntax() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct TestView {
                #[focus(invalid syntax)]
                field: Widget,
            }
        };

        let err = expand_focus(input)
            .expect_err("expected invalid focus attribute syntax to be rejected");
        assert!(
            err.to_string()
                .contains("Invalid syntax for #[focus] attribute")
        );
    }

    #[test]
    fn rejects_unknown_focus_attribute_key() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct TestView {
                #[focus(unknown = some_value)]
                field: Widget,
            }
        };

        let err = expand_focus(input).expect_err("expected unknown focus key to be rejected");
        assert!(
            err.to_string()
                .contains("Invalid argument for #[focus] attribute")
        );
    }

    #[test]
    /// UI-R-049 — `#[focus(nested)]` (bare, no `when`) is accepted.
    fn accepts_bare_nested_focus_attribute() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct TestView {
                #[focus(nested)]
                field: Widget,
            }
        };

        expand_focus(input).expect("expected #[focus(nested)] to be accepted");
    }

    #[test]
    /// UI-R-049 — `#[focus(nested, when = …)]` and `#[focus(when = …, nested)]` are both accepted.
    fn accepts_nested_with_when_either_argument_order() {
        let nested_then_when: syn::DeriveInput = syn::parse_quote! {
            struct TestView {
                #[focus(nested, when = self.flag)]
                field: Widget,
            }
        };
        expand_focus(nested_then_when).expect("expected #[focus(nested, when = …)] to be accepted");

        let when_then_nested: syn::DeriveInput = syn::parse_quote! {
            struct TestView {
                #[focus(when = self.flag, nested)]
                field: Widget,
            }
        };
        expand_focus(when_then_nested).expect("expected #[focus(when = …, nested)] to be accepted");
    }

    #[test]
    /// UI-R-049 — an unknown key alongside `nested` is still rejected like any other unknown key.
    fn rejects_nested_with_unknown_key() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct TestView {
                #[focus(nested, bogus = 1)]
                field: Widget,
            }
        };

        let err = expand_focus(input).expect_err("expected unknown key alongside nested to fail");
        assert!(
            err.to_string()
                .contains("Invalid argument for #[focus] attribute")
        );
    }

    #[test]
    fn rejects_focus_on_unnamed_field() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct TestView(
                #[focus]
                Widget,
            );
        };

        let err = expand_focus(input).expect_err("expected unnamed field to be rejected");
        assert!(
            err.to_string()
                .contains("FocusSwitch only works on named fields with ident")
        );
    }

    #[test]
    fn rejects_focus_derive_on_enum() {
        let input: syn::DeriveInput = syn::parse_quote! {
            enum TestEnum {
                #[focus]
                Variant,
            }
        };

        let err = expand_focus(input).expect_err("expected enum to be rejected");
        assert!(
            err.to_string()
                .contains("Focus can only be derived for structs")
        );
    }

    #[test]
    fn rejects_focusable_on_enum() {
        let input: syn::DeriveInput = syn::parse_quote! {
            enum TestEnum {
                Variant,
            }
        };

        let err = expand_focusable(TokenStream::new(), input)
            .expect_err("expected focusable on enum to be rejected");
        assert!(
            err.to_string()
                .contains("#[focusable] can only be applied to structs")
        );
    }

    #[test]
    fn rejects_focusable_on_tuple_struct() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct TestView(Widget, Widget);
        };

        let err = expand_focusable(TokenStream::new(), input)
            .expect_err("expected focusable on tuple struct to be rejected");
        assert!(
            err.to_string()
                .contains("#[focusable] only works on structs with named fields")
        );
    }

    #[test]
    /// UI-R-049 — `#[focusable]` (bare) and `#[focusable(nestable)]` are both accepted.
    fn focusable_accepts_bare_and_nestable_attr() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct TestView {
                field: Widget,
            }
        };
        expand_focusable(TokenStream::new(), input.clone())
            .expect("expected bare #[focusable] to be accepted");
        expand_focusable(quote::quote! { nestable }, input)
            .expect("expected #[focusable(nestable)] to be accepted");
    }

    #[test]
    /// UI-R-049 — an unknown `#[focusable(...)]` argument is rejected.
    fn focusable_rejects_unknown_attr_key() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct TestView {
                field: Widget,
            }
        };
        let err = expand_focusable(quote::quote! { bogus }, input)
            .expect_err("expected unknown #[focusable] argument to be rejected");
        assert!(
            err.to_string()
                .contains("Invalid argument for #[focusable] attribute")
        );
    }
}
