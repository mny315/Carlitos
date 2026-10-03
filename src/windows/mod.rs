//! Windows uses only OS media/desktop services; no GStreamer runtime is shipped.
pub(crate) mod decoder;
pub mod media;
pub(crate) mod output;
pub(crate) mod tempo;

/// COM apartments are thread local. Drop after every COM object on this thread.
pub struct Apartment(std::marker::PhantomData<std::rc::Rc<()>>);
impl Apartment {
    pub fn new() -> anyhow::Result<Self> {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self(std::marker::PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::Com::CoUninitialize();
        }
    }
}

pub(crate) fn initialize_media() -> anyhow::Result<()> {
    use std::sync::OnceLock;
    static STARTED: OnceLock<Result<(), String>> = OnceLock::new();
    STARTED
        .get_or_init(|| unsafe {
            use windows::Win32::Media::MediaFoundation::*;
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| {
                format!("Windows Media Foundation: {e}. Windows N requires the Media Feature Pack.")
            })
        })
        .clone()
        .map_err(anyhow::Error::msg)
}
