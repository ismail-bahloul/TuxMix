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

    /// The card is there, but its mixer isn't the one this backend
    /// speaks — in practice, a Babyface Pro running in Class Compliant
    /// mode on stock `snd-usb-audio`, whose control grammar
    /// (`Mic-AN1 Gain`, `Line-IN3-AN1`, …) this backend stopped
    /// targeting when it was rewritten for `snd-usb-babyface-pro`.
    /// Distinct from [`Error::DeviceNotFound`] so the UI can tell the
    /// user what to actually do about it.
    #[error("{card}: found, but its mixer is not the one TuxMix drives \
             (missing: {missing}). A Babyface Pro in Class Compliant mode \
             looks like this — TuxMix needs the snd-usb-babyface-pro driver \
             (proprietary mode). See babyface-pro-linux's README.")]
    UnsupportedDeviceMode { card: String, missing: String },

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
