use thiserror::Error;

/// Errors that can occur during device discovery or control.
#[derive(Error, Debug)]
pub enum Error {
    #[error("No RME device found matching: {model}")]
    DeviceNotFound { model: String },

    #[error("ALSA error: {0}")]
    #[cfg(feature = "alsa")]
    Alsa(#[from] alsa::Error),

    #[error("USB error: {0}")]
    #[cfg(feature = "usb")]
    Usb(#[from] tuxmix_usb::DeviceError),

    #[error("Mixer element not found: {0}")]
    MixerElementNotFound(String),

    #[error("Invalid channel: {0}")]
    InvalidChannel(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Scene captured on '{scene_model}' cannot be applied to '{device_model}'")]
    SceneModelMismatch {
        scene_model: String,
        device_model: String,
    },
}
