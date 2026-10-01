//! ESC/POS printing abstraction (plan section 6).
//!
//! Serves both non-fiscal receipts (contingency-mode provisional proof,
//! rejected-fiscal fallback) and DANFCE (the printed representation of an
//! authorized NFC-e) through the same pipeline: callers build a driver-
//! agnostic [`Receipt`] (see `commands::fiscal::print_receipt`) and hand it
//! to whichever [`Printer`] `build_printer()` selects, so business logic
//! never depends on a specific model/protocol (plan section 6's explicit
//! recommendation). Cash-drawer opening reuses the same ESC/POS "kick"
//! pulse sent through the printer's own connector — no extra hardware.
//!
//! Backed by the `escpos` crate. `PDV_PRINTER` selects the driver at
//! runtime (no rebuild needed, mirroring `config::backend_url`):
//!   - unset or `console`      -> prints to stdout (dev default; no
//!                                 hardware required to exercise the rest
//!                                 of the fiscal/printing flow)
//!   - `network:<host>:<port>` -> Ethernet thermal printer
//!   - `serial:<path>[:baud]`  -> RS232 or USB-to-serial (baud defaults
//!                                 to 9600)
//!   - `file:<path>`           -> a USB printer exposed as a raw device
//!                                 file (e.g. `/dev/usb/lp0` on Linux)
//!
//! True USB (vendor/product id) drivers need the `usb`/`native_usb`
//! feature, which pulls in libusb bindings this project doesn't enable by
//! default to keep the build free of a system-library dependency — most
//! "USB" thermal printers in the field are actually reachable via the
//! `file` or `serial` driver above. Add the feature in `Cargo.toml` if a
//! specific model needs true USB.

use std::path::Path;
use std::time::Duration;

use escpos::driver::{ConsoleDriver, Driver, FileDriver, NetworkDriver, SerialPortDriver};
use escpos::printer::Printer as EscPrinter;
use escpos::printer_options::PrinterOptions;
use escpos::utils::{
    CashDrawer, JustifyMode, Protocol, QRCodeCorrectionLevel, QRCodeModel, QRCodeOption,
};

const DRIVER_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_BAUD_RATE: u32 = 9600;

pub enum Justify {
    Left,
    Center,
}

/// One instruction in a receipt, independent of the physical driver that
/// will eventually render it.
pub enum ReceiptLine {
    Text {
        text: String,
        bold: bool,
        size: (u8, u8),
        justify: Justify,
    },
    QrCode {
        data: String,
    },
    Feed,
}

/// A full print job: an ordered list of lines, plus whether the cash
/// drawer should be kicked once printing finishes (plan section 1 —
/// "Cash drawer (kicked via printer)", used for cash-tendered sales).
pub struct Receipt {
    pub lines: Vec<ReceiptLine>,
    pub open_drawer: bool,
}

/// Fluent builder so callers (`commands::fiscal::print_receipt`) can
/// compose a receipt without touching `ReceiptLine`'s variants directly.
#[derive(Default)]
pub struct ReceiptBuilder {
    lines: Vec<ReceiptLine>,
}

impl ReceiptBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn header(mut self, text: impl Into<String>) -> Self {
        self.lines.push(ReceiptLine::Text {
            text: text.into(),
            bold: true,
            size: (2, 2),
            justify: Justify::Center,
        });
        self
    }

    pub fn centered(mut self, text: impl Into<String>) -> Self {
        self.lines.push(ReceiptLine::Text {
            text: text.into(),
            bold: false,
            size: (1, 1),
            justify: Justify::Center,
        });
        self
    }

    pub fn bold_line(mut self, text: impl Into<String>) -> Self {
        self.lines.push(ReceiptLine::Text {
            text: text.into(),
            bold: true,
            size: (1, 1),
            justify: Justify::Left,
        });
        self
    }

    pub fn line(mut self, text: impl Into<String>) -> Self {
        self.lines.push(ReceiptLine::Text {
            text: text.into(),
            bold: false,
            size: (1, 1),
            justify: Justify::Left,
        });
        self
    }

    pub fn feed(mut self) -> Self {
        self.lines.push(ReceiptLine::Feed);
        self
    }

    pub fn qrcode(mut self, data: impl Into<String>) -> Self {
        self.lines.push(ReceiptLine::QrCode { data: data.into() });
        self
    }

    pub fn build(self, open_drawer: bool) -> Receipt {
        Receipt {
            lines: self.lines,
            open_drawer,
        }
    }
}

/// Abstraction over a physical receipt printer, so business logic never
/// depends on a specific model/protocol.
pub trait Printer {
    fn print(&mut self, receipt: &Receipt) -> Result<(), String>;
}

/// Renders a [`Receipt`] onto any `escpos` driver — the same rendering
/// code path serves the console dev driver and every real hardware
/// driver, since only `D` differs.
pub struct EscPosPrinter<D: Driver> {
    inner: EscPrinter<D>,
}

impl<D: Driver> EscPosPrinter<D> {
    fn new(driver: D) -> Self {
        Self {
            inner: EscPrinter::new(driver, Protocol::default(), Some(PrinterOptions::default())),
        }
    }
}

impl<D: Driver> Printer for EscPosPrinter<D> {
    fn print(&mut self, receipt: &Receipt) -> Result<(), String> {
        self.inner.init().map_err(|e| e.to_string())?;

        for line in &receipt.lines {
            match line {
                ReceiptLine::Text {
                    text,
                    bold,
                    size,
                    justify,
                } => {
                    self.inner
                        .justify(match justify {
                            Justify::Left => JustifyMode::LEFT,
                            Justify::Center => JustifyMode::CENTER,
                        })
                        .map_err(|e| e.to_string())?
                        .bold(*bold)
                        .map_err(|e| e.to_string())?
                        .size(size.0, size.1)
                        .map_err(|e| e.to_string())?
                        .writeln(text)
                        .map_err(|e| e.to_string())?
                        .bold(false)
                        .map_err(|e| e.to_string())?
                        .size(1, 1)
                        .map_err(|e| e.to_string())?
                        .justify(JustifyMode::LEFT)
                        .map_err(|e| e.to_string())?;
                }
                ReceiptLine::QrCode { data } => {
                    self.inner
                        .qrcode_option(
                            data,
                            QRCodeOption::new(QRCodeModel::Model2, 6, QRCodeCorrectionLevel::M),
                        )
                        .map_err(|e| e.to_string())?;
                }
                ReceiptLine::Feed => {
                    self.inner.feed().map_err(|e| e.to_string())?;
                }
            }
        }

        if receipt.open_drawer {
            self.inner
                .cash_drawer(CashDrawer::Pin2)
                .map_err(|e| e.to_string())?;
        }

        self.inner.print_cut().map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Parses `PDV_PRINTER` and returns the matching driver, boxed behind the
/// [`Printer`] trait so callers never see which concrete `escpos` driver
/// backs it.
pub fn build_printer() -> Result<Box<dyn Printer>, String> {
    let target = std::env::var("PDV_PRINTER").unwrap_or_else(|_| "console".to_string());
    let mut parts = target.splitn(3, ':');
    let kind = parts.next().unwrap_or("console");

    match kind {
        "console" => Ok(Box::new(EscPosPrinter::new(ConsoleDriver::open(true)))),
        "network" => {
            let host = parts
                .next()
                .ok_or("PDV_PRINTER=network: needs a host, e.g. network:192.168.1.50:9100")?;
            let port: u16 = parts
                .next()
                .ok_or("PDV_PRINTER=network: needs a port, e.g. network:192.168.1.50:9100")?
                .parse()
                .map_err(|e| format!("invalid network printer port: {e}"))?;
            let driver = NetworkDriver::open(host, port, Some(DRIVER_TIMEOUT))
                .map_err(|e| e.to_string())?;
            Ok(Box::new(EscPosPrinter::new(driver)))
        }
        "serial" => {
            let path = parts
                .next()
                .ok_or("PDV_PRINTER=serial: needs a device path, e.g. serial:/dev/ttyUSB0")?;
            let baud = parts
                .next()
                .map(|s| s.parse::<u32>())
                .transpose()
                .map_err(|e| format!("invalid serial printer baud rate: {e}"))?
                .unwrap_or(DEFAULT_BAUD_RATE);
            let driver = SerialPortDriver::open(path, baud, Some(DRIVER_TIMEOUT))
                .map_err(|e| e.to_string())?;
            Ok(Box::new(EscPosPrinter::new(driver)))
        }
        "file" => {
            let path = parts
                .next()
                .ok_or("PDV_PRINTER=file: needs a device path, e.g. file:/dev/usb/lp0")?;
            let driver = FileDriver::open(Path::new(path)).map_err(|e| e.to_string())?;
            Ok(Box::new(EscPosPrinter::new(driver)))
        }
        other => Err(format!(
            "unknown PDV_PRINTER driver '{other}' (expected console, network:<host>:<port>, \
             serial:<path>[:baud], or file:<path>)"
        )),
    }
}
