use crate::{AppWindow, State, Theme};
use slint::{ComponentHandle, ModelRc, VecModel, platform::WindowEvent};
use std::{rc::Rc, time::Duration};

/// Visual fixtures use the real controls, with inert pairing data in demo mode.
pub fn sync_preview(window: &AppWindow, scene: String) {
    let weak = window.as_weak();
    slint::Timer::single_shot(Duration::from_millis(200), move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        let s = w.global::<State>();
        s.set_page(if scene == "library" { 0 } else { 3 });
        s.set_local_sync(scene != "disabled");
        s.set_sync_name("Linux · 123abc".into());
        s.set_sync_manual(scene == "manual" || scene == "qr");
        if scene == "qr" {
            // Inert compact invitation with two endpoints; no live key.
            let invitation = "CARLITOS://PAIR/2/VKVKVKVKVKVKVKVKVKVKVKVKVLGMZTGMZTGMZTGMZTGMZTGMZTGMBKABBKZTJLATAAA3GNA";
            s.set_sync_code("ZTGMZ-TGMZT-GMZTG-MZTGM-ZTGMZQ".into());
            s.set_sync_qr(carlitos::sync::qr_image(invitation).unwrap());
            s.set_sync_invitation(invitation.into());
        }
        if scene == "peers" || scene == "library" {
            s.set_sync_peers(ModelRc::from(Rc::new(VecModel::from(vec![
                carlitos::SyncPeer {
                    key: "1".into(),
                    name: "Android · 12abcd".into(),
                    online: true,
                },
                carlitos::SyncPeer {
                    key: "2".into(),
                    name: "Linux · 34efab".into(),
                    online: false,
                },
            ]))));
        }
        if scene == "error" {
            s.set_sync_error(true);
            s.set_sync_manual(true);
            s.set_sync_message(
                if w.global::<Theme>().get_english() {
                    "Could not pair devices. Check the network and renew the QR code"
                } else {
                    "Не удалось связать устройства. Проверьте сеть и обновите QR"
                }
                .into(),
            );
        }
        let weak = w.as_weak();
        slint::Timer::single_shot(Duration::from_millis(200), move || {
            if let Some(w) = weak.upgrade() {
                let size = w.window().size().to_logical(w.window().scale_factor());
                if scene != "library" {
                    w.window().dispatch_event(WindowEvent::PointerScrolled {
                        position: slint::LogicalPosition::new(size.width * 0.8, size.height * 0.45),
                        delta_x: 0.,
                        delta_y: -10000.,
                    });
                }
                w.window().dispatch_event(WindowEvent::PointerExited);
                sync_preview(&w, scene);
            }
        });
    });
}
