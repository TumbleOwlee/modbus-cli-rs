extern crate proc_macro;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{Field, Fields, Ident, Type, Visibility};

#[proc_macro_derive(Focus, attributes(focus))]
pub fn derive_focus(item: TokenStream) -> TokenStream {
    let mut input = syn::parse_macro_input!(item as syn::DeriveInput);
    let identifier = &input.ident;

    match &mut input.data {
        syn::Data::Struct(s) => {
            let mut idents = vec![];
            for field in s.fields.iter() {
                if field.attrs.iter().any(|attr| attr.path().is_ident("focus")) {
                    idents.push(field.ident.as_ref());
                }
            }

            let mut impl_handle = quote! {};
            let mut impl_previous = quote! {};
            let mut impl_next = quote! {};

            for i in 0..idents.len() {
                let from = *idents[i].as_ref().unwrap();
                let to_idx = (i + idents.len() - 1) % idents.len();
                let to = *idents[to_idx].as_ref().unwrap();
                impl_previous.extend(quote! {
                    #i => {
                        modbus_ui::traits::SetFocus::set_focused(&mut self.#from, false);
                        modbus_ui::traits::SetFocus::set_focused(&mut self.#to, true);
                        self.focus = #to_idx;
                    }
                });

                let from = *idents[i].as_ref().unwrap();
                let to_idx = (i + 1) % idents.len();
                let to = *idents[to_idx].as_ref().unwrap();
                impl_next.extend(quote! {
                    #i => {
                        modbus_ui::traits::SetFocus::set_focused(&mut self.#from, false);
                        modbus_ui::traits::SetFocus::set_focused(&mut self.#to, true);
                        self.focus = #to_idx;
                    }
                });

                impl_handle.extend(quote! {
                    #i => modbus_ui::traits::HandleEvents::handle_events(&mut self.#from, modifiers, code),
                });
            }

            for ts in [&mut impl_previous, &mut impl_next, &mut impl_handle] {
                ts.extend(quote! {
                    _ => {
                        unreachable!("Not reachable case");
                    }
                });
            }

            TokenStream::from(quote! {
                impl #identifier {
                    fn focus_previous(&mut self) {
                        match self.focus {
                            #impl_previous
                        }
                    }
                    fn focus_next(&mut self) {
                        match self.focus {
                            #impl_next
                        }
                    }
                }
                impl modbus_ui::traits::HandleEvents for #identifier {
                    fn handle_events(&mut self, modifiers: crossterm::event::KeyModifiers, code: crossterm::event::KeyCode) -> modbus_ui::EventResult {
                        match self.focus {
                            #impl_handle
                        }
                    }
                }
            })
        }
        _ => unimplemented!("State not implemented for type"),
    }
}

#[proc_macro_attribute]
pub fn focusable(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = syn::parse_macro_input!(item as syn::DeriveInput);

    match &mut input.data {
        syn::Data::Struct(s) => {
            let focus_field = Field {
                attrs: Vec::new(),
                mutability: syn::FieldMutability::None,
                vis: Visibility::Inherited,
                ident: Some(Ident::new("focus", Span::call_site())),
                colon_token: Some(Default::default()),
                ty: syn::parse_str::<Type>("usize").unwrap(),
            };

            match &mut s.fields {
                Fields::Named(named) => {
                    named.named.push(focus_field);
                }
                _ => {
                    unreachable!("FocusSwtich only works on named fields.");
                }
            }

            TokenStream::from(quote! {
                #input
            })
        }
        _ => unimplemented!("State not implemented for type"),
    }
}
