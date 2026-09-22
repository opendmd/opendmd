//! microSD over SPI, mounted as FAT at `/sdcard`.
//!
//! The DMDos carrier wires the card to a plain SPI socket, not the SDMMC
//! peripheral: VSPI/SPI2, `SCK=18 MISO=19 MOSI=23 CS=5` (measured, see
//! `hardware/PINOUT.md`). Card detect and write protect are not wired.
//!
//! Note that long filenames are a build-time concern, not a runtime one:
//! `CONFIG_FATFS_LFN_HEAP=y` and `CONFIG_FATFS_MAX_LFN=255` in
//! `sdkconfig.defaults` are what make `/gifs/NEW 2021/...` openable at all.

use anyhow::{Context, Result};
use esp_idf_hal::gpio::{AnyIOPin, AnyInputPin, AnyOutputPin};
use esp_idf_hal::peripheral::Peripheral;
use esp_idf_hal::sd::spi::SdSpiHostDriver;
use esp_idf_hal::sd::{SdCardConfiguration, SdCardDriver};
use esp_idf_hal::spi::config::DriverConfig;
use esp_idf_hal::spi::{Dma, SpiAnyPins, SpiDriver};
use esp_idf_svc::fs::fatfs::Fatfs;
use esp_idf_svc::io::vfs::MountedFatfs;
use opendmd_core::engine::Source;
use std::fs::File;
use std::io::{self, Read};

use crate::config;

type Host = SdSpiHostDriver<'static, &'static SpiDriver<'static>>;
type Card = SdCardDriver<Host>;

/// The SPI bus, owned for the life of the program.
///
/// The bus is set up exactly once (the SPI peripheral can only be taken once),
/// but mounting is retried forever, so the driver is leaked to `'static` and
/// each attempt borrows it. That is what lets the firmware sit and wait for a
/// card to be inserted instead of crash-looping.
pub struct SdBus {
    spi: &'static SpiDriver<'static>,
    cs: AnyOutputPin,
}

/// A mounted card. Dropping it unmounts, so keep it alive for the whole run.
pub struct SdCard {
    _fs: MountedFatfs<Fatfs<Card>>,
    /// Usable capacity in bytes, as reported by the card's CSD.
    pub capacity_bytes: u64,
}

impl SdBus {
    pub fn new<SPI: SpiAnyPins>(
        spi: impl Peripheral<P = SPI> + 'static,
        sck: AnyOutputPin,
        mosi: AnyOutputPin,
        miso: AnyIOPin,
        cs: AnyOutputPin,
    ) -> Result<SdBus> {
        let driver = SpiDriver::new(
            spi,
            sck,
            mosi,
            Some(miso),
            &DriverConfig::default().dma(Dma::Auto(4096)),
        )
        .context("SPI bus init failed")?;

        Ok(SdBus {
            // Leaked on purpose: the bus lives as long as the firmware does,
            // and `'static` keeps the retry loop free of lifetime gymnastics.
            spi: Box::leak(Box::new(driver)),
            cs,
        })
    }

    /// One mount attempt. Fails (rather than formatting) when no card is
    /// present, the card is unreadable, or it is not FAT — the Rust `Fatfs`
    /// API never formats, which is the `format_if_mount_failed = false` the
    /// brief asks for: a card that won't mount is a card to look at, not one
    /// to erase.
    pub fn mount(&mut self) -> Result<SdCard> {
        let host = SdSpiHostDriver::new(
            self.spi,
            // SAFETY: only the GPIO number is read out of the CS pin, and this
            // is the only place in the firmware that touches it. Re-handing it
            // to each retry cannot conflict with anything else.
            Some(unsafe { self.cs.clone_unchecked() }),
            AnyInputPin::none(),
            AnyInputPin::none(),
            AnyInputPin::none(),
            None,
        )
        .context("SD SPI host init failed")?;

        let mut cfg = SdCardConfiguration::new();
        cfg.speed_khz = config::SD_SPEED_KHZ;

        let card = SdCardDriver::new_spi(host, &cfg).context("no card, or card init failed")?;

        let raw = card.card();
        let capacity_bytes = (raw.csd.capacity as u64) * (raw.csd.sector_size as u64);

        let fatfs = Fatfs::new_sdcard(0, card).context("FAT driver registration failed")?;
        let mounted = MountedFatfs::mount(fatfs, config::SD_MOUNT, config::SD_MAX_FILES)
            .context("FAT mount failed")?;

        Ok(SdCard {
            _fs: mounted,
            capacity_bytes,
        })
    }
}

/// `opendmd_core::engine::Source` over the mounted card.
///
/// Manifest paths are card-absolute (`/gifs/Consoles/X.gif`); joining them
/// onto the mount point is this type's whole job.
pub struct SdSource;

impl Source for SdSource {
    fn open(&self, path: &str) -> io::Result<Box<dyn Read>> {
        let full = format!("{}{path}", config::SD_MOUNT);
        Ok(Box::new(File::open(full)?))
    }
}
