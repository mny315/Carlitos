use crate::app::Command;
use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};

// Embed the white tray mark so development and portable builds also work
// without an installed icon theme. StatusNotifierItem expects straight ARGB.
static TRAY_ICON: LazyLock<ksni::Icon> = LazyLock::new(|| {
    let pixels = slint::Image::load_from_svg_data(include_bytes!(
        "../../data/icons/io.github.mny315.Carlitos-symbolic.svg"
    ))
    .expect("valid embedded tray SVG")
    .to_rgba8()
    .expect("rasterized tray SVG");
    let mut data = pixels.as_bytes().to_vec();
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1);
    }
    ksni::Icon {
        width: pixels.width() as i32,
        height: pixels.height() as i32,
        data,
    }
});

pub(super) struct Tray {
    pub(super) tx: Sender<Command>,
    pub(super) available: Arc<AtomicBool>,
}
impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "carlitos".into()
    }
    fn title(&self) -> String {
        "Carlitos".into()
    }
    fn icon_name(&self) -> String {
        // Hosts prefer a themed name over IconPixmap, including stale installed
        // artwork. Always use the mark embedded in this running build.
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![TRAY_ICON.clone()]
    }
    fn activate(&mut self, _: i32, _: i32) {
        let _ = self.tx.send(Command::Show);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem {
                label: crate::app::text("Показать Carlitos", "Show Carlitos").into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Command::Show);
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: crate::app::text("Воспроизведение / пауза", "Play / pause").into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Command::Toggle);
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: crate::app::text("Выйти", "Quit").into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Command::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        if self.available.swap(false, Ordering::SeqCst) {
            let _ = self.tx.send(Command::Show);
        }
        true
    }
}
