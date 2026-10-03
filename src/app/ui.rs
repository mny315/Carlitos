use super::{Event, Handle, Options, view::View};
use crate::{AppWindow, State};
use anyhow::Result;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

/// Shared UI/controller startup. Backend selection and OS integration stay in
/// the platform entry points; all windows use the same models and callbacks.
pub struct Ui {
    pub window: AppWindow,
    pub app: Handle,
    pub view: Rc<RefCell<View>>,
    covers: async_channel::Receiver<super::view::CoverResult>,
}

impl Ui {
    pub fn new(mut options: Options) -> Result<Self> {
        options.settings.validate()?;
        crate::i18n::configure(&options.settings.language);
        let window = AppWindow::new()?;
        #[cfg(any(target_os = "linux", windows))]
        {
            window.window().set_size(slint::LogicalSize::new(
                options.settings.window_width as f32,
                options.settings.window_height as f32,
            ));
            window.window().set_maximized(options.settings.maximized);
        }
        window.global::<State>().set_demo(options.demo);
        window.global::<State>().set_windows(cfg!(windows));
        window
            .global::<State>()
            .set_android(cfg!(target_os = "android"));
        let settings = options.settings.clone();
        let app = Handle::start(options)?;
        let (view, covers) = View::new(&window, app.tx.clone(), settings);
        Ok(Self {
            window,
            app,
            view,
            covers,
        })
    }

    pub fn connect(&self, mut platform_event: impl FnMut(&Event) + 'static) -> Result<()> {
        let receiver = self.app.events.clone();
        let view = Rc::downgrade(&self.view);
        slint::spawn_local(async move {
            while let Ok(event) = receiver.recv().await {
                platform_event(&event);
                if let Some(view) = view.upgrade() {
                    #[cfg(target_os = "android")]
                    if matches!(event, Event::AndroidRefresh) {
                        let mut library = false;
                        let events = crate::android::runtime::take_events();
                        for event in events.snapshots {
                            library |= matches!(event, Event::Library(_));
                            view.borrow_mut().event(event);
                        }
                        if library {
                            crate::android::ui::restore();
                        }
                        // Picker/worker results target the restored editor,
                        // and completed imports must override the saved page.
                        for event in events.pending {
                            view.borrow_mut().event(event);
                        }
                        continue;
                    }
                    view.borrow_mut().event(event);
                } else {
                    break;
                }
            }
        })?;
        let covers = self.covers.clone();
        let view = Rc::downgrade(&self.view);
        slint::spawn_local(async move {
            while let Ok(cover) = covers.recv().await {
                if let Some(view) = view.upgrade() {
                    view.borrow_mut().cover(cover);
                } else {
                    break;
                }
            }
        })?;
        Ok(())
    }
}
