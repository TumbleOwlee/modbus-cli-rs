//! OCPP creation dialog wrapper. Implements [`SetupView`] over [`OcppSetupDialog`] and, on
//! confirm, builds the matching view for the chosen role (client → full CS view, server →
//! placeholder).

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::EventResult;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::module::ocpp::client::build_client_view;
use crate::module::ocpp::config::device::OcppDeviceConfig;
use crate::module::ocpp::config::session::{OcppModuleSpec, OcppProtocol, OcppRole, OcppSpec};
use crate::module::ocpp::server::build_server_view;
use crate::module::ocpp::setup_dialog::OcppSetupDialog;
use crate::module::type_descriptor::{ModuleViewFactory, SetupView};

/// Setup dialog for the OCPP module type.
pub struct OcppSetupView {
    dialog: OcppSetupDialog,
}

impl OcppSetupView {
    pub fn new() -> Self {
        Self {
            dialog: OcppSetupDialog::new(),
        }
    }
}

impl SetupView for OcppSetupView {
    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.dialog.render(area, buf);
    }

    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        self.dialog.handle_events(modifiers, code)
    }

    fn focus_next(&mut self) {
        self.dialog.focus_next();
    }

    fn focus_previous(&mut self) {
        self.dialog.focus_previous();
    }

    fn close_requested(&mut self) -> bool {
        self.dialog.take_close_request()
    }

    fn confirm(&self) -> Option<(String, ModuleViewFactory)> {
        let spec = self.dialog.resolve().ok()?;
        let path = self.dialog.config_path();
        let name = spec.name.clone();
        let device = build_device(&self.dialog, &spec, &path);

        // Reconcile the runtime spec with the (possibly file-sourced) device fields + endpoint.
        let module = OcppModuleSpec::from_spec(&spec, &path);
        let spec = OcppSpec::from_parts(&module, &device);

        let factory: ModuleViewFactory = match device.role {
            OcppRole::Client => Box::new(move || build_client_view(spec, path, device)),
            OcppRole::Server => Box::new(move || build_server_view(spec, path, device)),
        };
        Some((name, factory))
    }
}

/// Assemble the device config for [`OcppSetupView::confirm`]: an existing file at `path` is
/// authoritative (its scripts, and — to avoid clobbering — its version/role/timeout); otherwise
/// build it from the dialog's selections with no scripts yet. `extra_headers` (OC-R-117/118/119,
/// UI-R-059) always comes from the dialog's working list, regardless of the file/fresh split
/// above — it has no file fallback the way security does, since a fresh dialog's table starts
/// empty either way.
fn build_device(dialog: &OcppSetupDialog, spec: &OcppSpec, path: &str) -> OcppDeviceConfig {
    let mut device = if path.is_empty() {
        OcppDeviceConfig::from_spec(spec, Vec::new())
    } else {
        match crate::config::load_ocpp_device(path) {
            Ok(mut loaded) => {
                apply_security_precedence(&mut loaded, spec);
                loaded
            }
            Err(_) => OcppDeviceConfig::from_spec(spec, Vec::new()),
        }
    };
    device.extra_headers = dialog.extra_headers();
    device
}

/// Decide which security section wins when merging a loaded device config with the dialog's
/// resolved spec. The dialog only exposes security controls for `wss://`, so a `ws://` selection
/// must not silently wipe out a security section already present in the loaded file: the file's
/// section is left untouched. For `wss://` the dialog is authoritative and overwrites it.
fn apply_security_precedence(loaded: &mut OcppDeviceConfig, spec: &OcppSpec) {
    if spec.protocol == OcppProtocol::Wss {
        loaded.security = spec.security.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::ocpp::config::device::OcppSecurityConfig;
    use crate::module::ocpp::config::session::OcppVersion;
    use crossterm::event::{KeyCode, KeyModifiers};

    // `SetupView::close_requested`'s default trait method must be overridden here to delegate to
    // the dialog's close-confirm popup; without the override the creation overlay's Esc→confirm
    // flow silently does nothing for an OCPP module setup.
    #[test]
    /// UI-R-023 — the OCPP setup delegates close-requested to the dialog's close-request flag.
    fn ut_close_requested_delegates_to_dialog_take_close_request() {
        let mut sv = OcppSetupView::new();
        assert!(!sv.close_requested());
        sv.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        sv.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(sv.close_requested());
        assert!(!sv.close_requested(), "flag must clear after take");
    }

    fn base_spec(protocol: OcppProtocol) -> OcppSpec {
        OcppSpec {
            name: "cs-1".into(),
            version: OcppVersion::V1_6,
            role: OcppRole::Client,
            protocol,
            ip: "127.0.0.1".into(),
            port: 9000,
            path: String::new(),
            timeout_ms: None,
            reconnect: None,
            security: OcppSecurityConfig {
                username: Some("dialog-user".into()),
                ..Default::default()
            },
        }
    }

    #[test]
    /// A ws setup preserves the loaded security config on resolve.
    fn ut_ws_preserves_loaded_security() {
        let mut loaded = OcppDeviceConfig {
            security: OcppSecurityConfig {
                tls: crate::module::ocpp::config::device::OcppTlsConfig {
                    client: ferrowl_util::tls::ClientTlsPolicy::Tls {
                        verification: ferrowl_util::tls::CertVerification::RootStore {
                            extra_ca_files: vec!["existing-ca.pem".into()],
                        },
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let spec = base_spec(OcppProtocol::Ws);
        apply_security_precedence(&mut loaded, &spec);
        assert_eq!(
            loaded.security.tls.client,
            ferrowl_util::tls::ClientTlsPolicy::Tls {
                verification: ferrowl_util::tls::CertVerification::RootStore {
                    extra_ca_files: vec!["existing-ca.pem".into()],
                }
            }
        );
        assert_eq!(loaded.security.username, None);
    }

    #[test]
    /// A wss setup resolves the dialog's security config over the loaded one.
    fn ut_wss_overwrites_loaded_security_with_dialog() {
        let mut loaded = OcppDeviceConfig {
            security: OcppSecurityConfig {
                tls: crate::module::ocpp::config::device::OcppTlsConfig {
                    client: ferrowl_util::tls::ClientTlsPolicy::Tls {
                        verification: ferrowl_util::tls::CertVerification::RootStore {
                            extra_ca_files: vec!["existing-ca.pem".into()],
                        },
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let spec = base_spec(OcppProtocol::Wss);
        apply_security_precedence(&mut loaded, &spec);
        assert_eq!(loaded.security, spec.security);
    }

    #[test]
    fn ut_render_and_focus_delegate_to_dialog() {
        let mut sv = OcppSetupView::new();
        sv.focus_next();
        sv.focus_previous();
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        sv.render(area, &mut buf);
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!text.trim().is_empty());
    }

    #[test]
    fn ut_confirm_resolves_and_builds_a_working_factory() {
        let sv = OcppSetupView::new();
        if let Some((name, factory)) = sv.confirm() {
            assert!(!name.is_empty());
            let _view = factory();
        }
    }

    #[test]
    /// OC-R-117, UI-R-059 — `confirm` (exercised here through the same public
    /// `handle_events`/`confirm` surface the app drives) succeeds with a populated headers
    /// table, and the device it composes carries that list — not just the resolved `OcppSpec`,
    /// which has no headers field of its own.
    ///
    /// `ModuleView` (what `confirm`'s factory produces) deliberately exposes no way to read a
    /// built view's internal `device` back out — no downcast, no accessor — so the composed
    /// device itself is checked via `build_device`, the same private helper `confirm` calls
    /// with the same dialog/spec/path, rather than by inspecting the opaque view. This mirrors
    /// the existing precedent for testing this function's other composition step
    /// (`ut_ws_preserves_loaded_security`/`ut_wss_overwrites_loaded_security_with_dialog` test
    /// `apply_security_precedence` the same way).
    fn ut_confirm_carries_dialog_extra_headers_onto_device() {
        let mut sv = OcppSetupView::new();
        for c in "cs-1".chars() {
            sv.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }
        sv.dialog.extra_headers = vec![ferrowl_ocpp::HeaderDef::new("X-Tenant", "acme-1").unwrap()];

        let (name, factory) = sv.confirm().expect("a named client dialog resolves");
        assert_eq!(name, "cs-1");
        let _view = factory();

        let spec = sv.dialog.resolve().expect("still resolves after confirm");
        let device = build_device(&sv.dialog, &spec, &sv.dialog.config_path());
        assert_eq!(device.extra_headers.len(), 1);
        assert_eq!(device.extra_headers[0].name, "X-Tenant");
    }

    #[test]
    /// UI-R-059, UI-R-095 — a header added via real keystrokes (type name, Tab, type value, Enter to add
    /// — not by poking `dialog.extra_headers` directly) must still be present when the dialog is
    /// confirmed, even if the very next key after adding is another bare Enter (the natural next
    /// keystroke a user presses to close the dialog).
    fn ut_header_added_via_keystrokes_survives_a_second_enter_and_confirm() {
        let mut sv = OcppSetupView::new();
        for c in "cs-1".chars() {
            sv.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }

        // Tab from Name to HeaderNameInput: ConfigPath, Version, Role, Reconnect, Ip, Port,
        // Path, then HeaderNameInput — 8 hops (fresh dialog defaults to ws/client, headers
        // table hidden while empty; Protocol is a derived read-only display, not in the focus
        // cycle, OC-R-127). `SetupView` routes Tab via `focus_next`, not through
        // `handle_events` (the app shell calls it directly on a Tab key — see `close_requested`'s
        // sibling methods on this trait), so drive focus that way here too.
        for _ in 0..8 {
            sv.focus_next();
        }
        for c in "X-Tenant".chars() {
            sv.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }
        sv.focus_next(); // -> HeaderValueInput
        for c in "acme-1".chars() {
            sv.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }
        sv.handle_events(KeyModifiers::NONE, KeyCode::Enter); // add the header
        assert_eq!(
            sv.dialog.extra_headers.len(),
            1,
            "the header must land in the dialog's working list right after Enter-to-add"
        );

        // The literal reported next keystroke: a second bare Enter, still focused in the
        // headers-add cluster. The app shell (`App::handle_dialog_key`) only calls `confirm()`
        // when `handle_events` reports the key `Unhandled` — so this must not be consumed
        // trying (and failing) to add an empty header, or the dialog could never be confirmed
        // from this focus state at all.
        assert!(
            matches!(
                sv.handle_events(KeyModifiers::NONE, KeyCode::Enter),
                EventResult::Unhandled(KeyModifiers::NONE, KeyCode::Enter)
            ),
            "a second bare Enter with nothing typed must fall through to confirm, not be \
             swallowed trying to add an empty header"
        );

        let (name, factory) = sv.confirm().expect("a named client dialog resolves");
        assert_eq!(name, "cs-1");
        let _view = factory();

        let spec = sv.dialog.resolve().expect("still resolves after confirm");
        let device = build_device(&sv.dialog, &spec, &sv.dialog.config_path());
        assert_eq!(device.extra_headers.len(), 1);
        assert_eq!(device.extra_headers[0].name, "X-Tenant");
    }
}
