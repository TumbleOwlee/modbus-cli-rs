//! Boundary between the binary and the [`ferrowl_templates`] crate that owns the bundled Lua script
//! templates (SC-R-036). The templates carry their own [`TemplateContext`]; this module maps that to
//! the binary's [`ScriptContext`] and exposes the `ScriptContext`-keyed lookup the dialogs use.

use ferrowl_templates::TemplateContext;

pub use ferrowl_templates::{ScriptTemplate, by_name};

use crate::dialog::lua_help::ScriptContext;

impl From<TemplateContext> for ScriptContext {
    fn from(c: TemplateContext) -> Self {
        match c {
            TemplateContext::Modbus => ScriptContext::Modbus,
            TemplateContext::OcppClient => ScriptContext::OcppClient,
            TemplateContext::OcppServer => ScriptContext::OcppServer,
            TemplateContext::Session => ScriptContext::Session,
        }
    }
}

/// The templates applicable to `ctx`, in declaration order.
pub fn templates(ctx: ScriptContext) -> Vec<&'static ScriptTemplate> {
    ferrowl_templates::all()
        .iter()
        .filter(|t| t.contexts.iter().any(|&c| ScriptContext::from(c) == ctx))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTEXTS: [ScriptContext; 4] = [
        ScriptContext::Modbus,
        ScriptContext::OcppClient,
        ScriptContext::OcppServer,
        ScriptContext::Session,
    ];

    #[test]
    /// SC-R-063 — `templates(ctx)` returns exactly the templates declaring `ctx`.
    fn ut_templates_filtered_by_context() {
        for ctx in CONTEXTS {
            for template in templates(ctx) {
                assert!(
                    template
                        .contexts
                        .iter()
                        .any(|&c| ScriptContext::from(c) == ctx),
                    "'{}' listed under {ctx:?} without declaring it",
                    template.name
                );
            }
        }
        let modbus: Vec<_> = templates(ScriptContext::Modbus)
            .iter()
            .map(|t| t.name)
            .collect();
        assert!(modbus.contains(&"register-ramp"));
        assert!(!modbus.contains(&"power-report"));
    }

    #[test]
    fn ut_every_context_has_a_template() {
        for ctx in CONTEXTS {
            assert!(!templates(ctx).is_empty(), "{ctx:?} has no template");
        }
    }
}
