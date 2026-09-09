//! The bundled Lua script templates (SC-R-036): starting points a user can drop into a module's
//! script list from the script dialog's template browser.
//!
//! A template is *not* a script: its code is compiled into the binary and only ever **copied** into
//! a script list — nothing reads template code from disk at run time, so scripts stay stored inline
//! in the config files (SC-R-023).
//!
//! The `TEMPLATES` array is generated at build time by `build.rs`, which walks `templates/` and
//! derives each entry's name (file stem), context (parent directory), description (leading
//! `-- description:` header), and code (the file minus that header). Adding a template is dropping a
//! `.lua` in the right context directory; a file missing its description header fails the build.

/// Which script context a template applies to. The binary maps this to its own `ScriptContext`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateContext {
    Modbus,
    OcppClient,
    OcppServer,
    Session,
}

/// One bundled template: its display name, a one-line description, the script contexts it applies
/// to, and its Lua code.
#[derive(Debug, PartialEq, Eq)]
pub struct ScriptTemplate {
    pub name: &'static str,
    pub description: &'static str,
    pub contexts: &'static [TemplateContext],
    pub code: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/templates_generated.rs"));

/// Every bundled template, in generated order (context group, then name).
pub fn all() -> &'static [ScriptTemplate] {
    TEMPLATES
}

/// The bundled template with `name`, if any.
pub fn by_name(name: &str) -> Option<&'static ScriptTemplate> {
    TEMPLATES.iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// SC-R-037 — every bundled template's code body loads in the Lua runtime; a syntax error in a
    /// template is a test failure, never something a user meets after inserting it.
    fn ut_every_template_compiles() {
        for template in all() {
            let ctx = ferrowl_lua::ContextBuilder::<String>::default()
                .with_stdlib()
                .with_script(template.name.to_string(), template.code)
                .build();
            assert!(
                ctx.is_ok(),
                "template '{}' failed to load: {:?}",
                template.name,
                ctx.err()
            );
        }
    }

    #[test]
    /// SC-R-063 — every bundled template (SC-R-036) has a name, a one-line description, a Lua
    /// code body, and a non-empty set of script contexts it applies to.
    fn ut_every_template_has_name_description_code_and_contexts() {
        for template in all() {
            assert!(!template.name.is_empty(), "template with empty name");
            assert!(
                !template.description.is_empty(),
                "template '{}' has an empty description",
                template.name
            );
            assert!(
                !template.description.contains('\n'),
                "template '{}' description is not one line",
                template.name
            );
            assert!(
                !template.code.is_empty(),
                "template '{}' has no code body",
                template.name
            );
            assert!(
                !template.contexts.is_empty(),
                "template '{}' applies to no context",
                template.name
            );
        }
    }

    #[test]
    fn ut_by_name_finds_bundled_template() {
        assert!(by_name("power-report").is_some());
        assert!(by_name("nope").is_none());
    }
}
