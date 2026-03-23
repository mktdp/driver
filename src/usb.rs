//! USB driver for DigitalPersona U.are.U 4500 and compatible scanners.
//!
//! This module owns the raw USB communication: device enumeration, the
//! initialisation state-machine, finger-presence polling, and raw image
//! capture.  It is the **only** module that touches `rusb`.
//!
//! Protocol details are reverse-engineered from the libfprint `uru4000.c`
//! driver (Copyright © 2007-2008 Daniel Drake, 2012 Timo Teräs).

use std::time::{Duration, Instant};

use crate::error::{FpError, Result};

macro_rules! debug_log {
    ($($arg:tt)*) => {
        if cfg!(feature = "debug-logging") {
            eprintln!($($arg)*);
        }
    };
}

// ── USB identifiers ────────────────────────────────────────────────

/// Vendor ID for DigitalPersona / ZKSoftware devices.
const VID: u16 = 0x05ba;
/// Product ID for U.are.U 4000B / 4500 / Biokey 200.
const PID: u16 = 0x000a;

// ── Endpoints (confirmed via `lsusb -v`) ───────────────────────────

/// Interrupt-IN endpoint — finger presence events (2 bytes, big-endian).
const EP_INTR: u8 = 0x81;
/// Bulk-IN endpoint — image data.
const EP_DATA: u8 = 0x82;

// ── Control transfer parameters ────────────────────────────────────

/// Vendor-specific bRequest used for all register reads/writes.
const USB_RQ: u8 = 0x04;
/// `bmRequestType` for a vendor read (device-to-host).
const CTRL_IN: u8 = 0xC0; // direction=IN | type=Vendor | recipient=Device
/// `bmRequestType` for a vendor write (host-to-device).
const CTRL_OUT: u8 = 0x40; // direction=OUT | type=Vendor | recipient=Device
/// Timeout for all control transfers (milliseconds).
const CTRL_TIMEOUT: Duration = Duration::from_millis(5_000);

// ── Registers ──────────────────────────────────────────────────────

/// Hardware status register.
const REG_HWSTAT: u16 = 0x07;
/// Scramble-data index register (write key_number + seed).
const REG_SCRAMBLE_DATA_INDEX: u16 = 0x33;
/// Scramble-data key register (read 4-byte key).
const REG_SCRAMBLE_DATA_KEY: u16 = 0x34;
/// Operating-mode register.
const REG_MODE: u16 = 0x4e;
/// Device info / firmware version register (16 bytes).
const REG_DEVICE_INFO: u16 = 0xf0;

// ── Modes ──────────────────────────────────────────────────────────

/// Initial/reset mode.
const MODE_INIT: u8 = 0x00;
/// Wait for finger placed on sensor.
const MODE_AWAIT_FINGER_ON: u8 = 0x10;
/// Wait for finger removed from sensor.
const MODE_AWAIT_FINGER_OFF: u8 = 0x12;
/// Start image capture.
const MODE_CAPTURE: u8 = 0x20;
/// Power-off mode (used during deactivation).
const _MODE_OFF: u8 = 0x70;

// ── Interrupt data types ───────────────────────────────────────────

/// Scan power is on — sent after successful init.
const IRQDATA_SCANPWR_ON: u16 = 0x56aa;
/// A finger has been placed on the sensor.
const IRQDATA_FINGER_ON: u16 = 0x0101;
/// The finger has been removed from the sensor.
const IRQDATA_FINGER_OFF: u16 = 0x0200;

// ── Image dimensions ──────────────────────────────────────────────

/// Width of the captured image in pixels.
pub const IMAGE_WIDTH: usize = 384;
/// Height of the captured image in pixels.
pub const IMAGE_HEIGHT: usize = 289;
/// Expected pixel count for a valid, deframed image.
pub const IMAGE_SIZE: usize = IMAGE_WIDTH * IMAGE_HEIGHT;

/// Default stable-contact window after first finger-on IRQ.
///
/// Capture starts only after this window passes without a finger-off IRQ.
const DEFAULT_FINGER_DEBOUNCE_MS: u64 = 0;
/// Default wait after finger-on before entering capture mode.
///
/// This lets pressure/contact settle so we do not capture too early.
const DEFAULT_CAPTURE_SETTLE_MS: u64 = 0;
/// Default wait after entering capture mode before bulk read.
///
/// Keep this at 0 by default to avoid visible LED flicker on some units.
const DEFAULT_CAPTURE_HOLD_MS: u64 = 0;
/// Hard clamp for env-configured delays to avoid pathological values.
const MAX_CAPTURE_DELAY_MS: u64 = 2_000;
/// Env var: stable-contact debounce window after finger-on.
const ENV_FINGER_DEBOUNCE_MS: &str = "FP_FINGER_DEBOUNCE_MS";
/// Env var: settle delay after finger-on, before MODE_CAPTURE.
const ENV_CAPTURE_SETTLE_MS: &str = "FP_CAPTURE_SETTLE_MS";
/// Env var: hold delay after MODE_CAPTURE, before bulk read.
const ENV_CAPTURE_HOLD_MS: &str = "FP_CAPTURE_HOLD_MS";

/// Length of the `uru4k_image` header in bytes.
///
/// Layout (from libfprint):
/// ```text
///   [0..4]   unknown
///   [4..6]   num_lines (u16 LE)
///   [6]      key_number (u8)
///   [7..16]  unknown
///   [16..46] block_info (15 × 2 bytes: flags + num_lines)
///   [46..64] unknown
/// ```
pub const IMAGE_HEADER_LEN: usize = 64;

/// Interrupt packet length (2 bytes — big-endian u16 type code).
const IRQ_LENGTH: usize = 64; // device always sends 64 bytes; type is in first 2

// ── Bulk read buffer ──────────────────────────────────────────────

/// Maximum size we request in a single bulk read.
/// Header (64) + full image (384 × 289).
const DATABLK_RQLEN: usize = IMAGE_HEADER_LEN + IMAGE_SIZE;

// ── Opaque device handle ──────────────────────────────────────────

/// Opaque handle to an opened fingerprint scanner.
///
/// Constructed by [`open()`] and destroyed by [`close()`].  Not
/// `Send` or `Sync` — callers must serialise access.
pub struct FpDevice {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
    interface: u8,
}

// ── Public API ─────────────────────────────────────────────────────

/// Open the first available U.are.U 4500 scanner on the USB bus.
///
/// Finds the device by VID/PID, locates the vendor-class interface
/// (class = subclass = protocol = 0xFF), claims it, detaches any
/// kernel driver if necessary, and runs the init state-machine.
pub fn open() -> Result<FpDevice> {
    let handle = rusb::open_device_with_vid_pid(VID, PID).ok_or(FpError::DeviceNotFound)?;

    let device = handle.device();
    let config = device.active_config_descriptor().map_err(FpError::UsbIo)?;

    // Find the vendor-specific interface (class=subclass=protocol=0xFF).
    let iface = config
        .interfaces()
        .find(|i| {
            i.descriptors().any(|d| {
                d.class_code() == 0xFF && d.sub_class_code() == 0xFF && d.protocol_code() == 0xFF
            })
        })
        .ok_or(FpError::DeviceNotFound)?;

    let iface_num = iface.number();

    // Detach kernel driver if attached (e.g. usbhid).
    if handle.kernel_driver_active(iface_num).unwrap_or(false) {
        handle
            .detach_kernel_driver(iface_num)
            .map_err(FpError::UsbIo)?;
    }

    handle.claim_interface(iface_num).map_err(FpError::UsbIo)?;

    let mut dev = FpDevice {
        handle,
        interface: iface_num,
    };

    // Run the init state-machine.
    init_device(&mut dev)?;

    Ok(dev)
}

/// Wait for a finger to be placed on the sensor, capture the raw
/// image, and return the deframed pixel buffer.
///
/// `timeout_ms`: Maximum time to wait for finger presence.
///               `0` means wait indefinitely.
///
/// The returned `Vec<u8>` contains `IMAGE_WIDTH × IMAGE_HEIGHT`
/// bytes of 8-bit grayscale, already deframed and descrambled.
pub fn scan(dev: &mut FpDevice, timeout_ms: u32) -> Result<Vec<u8>> {
    debug_log!("[usb::scan] === START (timeout={}ms) ===", timeout_ms);
    let timing = capture_timing();

    // 1. Wait for finger presence.
    debug_log!("[usb::scan] step 1: waiting for finger...");
    wait_for_stable_finger_on(dev, timeout_ms, timing.finger_debounce_ms)?;
    debug_log!("[usb::scan] step 1: finger detected!");
    if timing.settle_ms > 0 {
        debug_log!(
            "[usb::scan] step 1b: settle delay {}ms before capture",
            timing.settle_ms
        );
        std::thread::sleep(Duration::from_millis(timing.settle_ms));
    }

    // 2. Set capture mode and read image with retry.
    //    The device sometimes sends a ZLP (zero-length packet) on
    //    the first bulk read after MODE_CAPTURE, especially on
    //    subsequent captures.  libfprint handles this by retrying
    //    the bulk read (IMAGING_CAPTURE loop) without re-writing
    //    the mode register.  We do the same.
    debug_log!("[usb::scan] step 2: setting MODE_CAPTURE...");
    write_reg(dev, REG_MODE, MODE_CAPTURE)?;
    if timing.hold_ms > 0 {
        debug_log!(
            "[usb::scan] step 2a: capture hold {}ms before bulk read",
            timing.hold_ms
        );
        std::thread::sleep(Duration::from_millis(timing.hold_ms));
    }

    let mut raw = bulk_read_image_with_retry(dev)?;
    debug_log!("[usb::scan] step 2: got {} bytes", raw.len());

    // 2b. Decrypt the image if it is encrypted.
    //     The device may encrypt pixel data with an LFSR stream cipher.
    //     We detect this via a variance metric on the first two image
    //     lines, then read the decryption key from the device over USB.
    decrypt_image_data(dev, &mut raw)?;

    // 3. Wait for the finger to be lifted before returning.
    //    This matches the libfprint state machine:
    //    CAPTURE → AWAIT_FINGER_OFF → (return) → AWAIT_FINGER_ON → CAPTURE
    //    We do NOT set MODE_INIT between captures — that confuses the device.
    debug_log!("[usb::scan] step 3: waiting for finger OFF...");
    match wait_for_finger_off(dev, 5_000) {
        Ok(()) => debug_log!("[usb::scan] step 3: finger lifted OK"),
        Err(e) => debug_log!(
            "[usb::scan] step 3: finger-off wait failed: {} (continuing anyway)",
            e
        ),
    }

    debug_log!("[usb::scan] === DONE ({} bytes) ===", raw.len());

    Ok(raw)
}

/// Close the device, releasing the USB interface.
pub fn close(dev: FpDevice) {
    // Best-effort: set mode to init, then power down.
    let _ = write_reg_inner(&dev.handle, REG_MODE, MODE_INIT);
    let _ = write_reg_inner(&dev.handle, REG_HWSTAT, 0x80);
    let _ = dev.handle.release_interface(dev.interface);
    // `dev` is dropped here — the `DeviceHandle` is closed.
}

// ── Internals ──────────────────────────────────────────────────────

/// Write a single byte to a device register via vendor control transfer.
fn write_reg(dev: &FpDevice, reg: u16, value: u8) -> Result<()> {
    write_reg_inner(&dev.handle, reg, value)
}

fn write_reg_inner(
    handle: &rusb::DeviceHandle<rusb::GlobalContext>,
    reg: u16,
    value: u8,
) -> Result<()> {
    handle.write_control(
        CTRL_OUT,
        USB_RQ,
        reg, // wValue = register address
        0,   // wIndex = 0
        &[value],
        CTRL_TIMEOUT,
    )?;
    Ok(())
}

/// Write multiple bytes to consecutive device registers via vendor control transfer.
fn write_regs(dev: &FpDevice, first_reg: u16, data: &[u8]) -> Result<()> {
    dev.handle.write_control(
        CTRL_OUT,
        USB_RQ,
        first_reg, // wValue = first register address
        0,         // wIndex = 0
        data,
        CTRL_TIMEOUT,
    )?;
    Ok(())
}

/// Read `len` bytes from a device register via vendor control transfer.
fn read_reg(dev: &FpDevice, reg: u16, len: u16) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len as usize];
    let n = dev.handle.read_control(
        CTRL_IN,
        USB_RQ,
        reg, // wValue = register address
        0,   // wIndex = 0
        &mut buf,
        CTRL_TIMEOUT,
    )?;
    buf.truncate(n);
    Ok(buf)
}

/// Capture timing knobs loaded from environment.
///
/// - `FP_CAPTURE_SETTLE_MS`: delay after finger-on before MODE_CAPTURE
/// - `FP_CAPTURE_HOLD_MS`: delay after MODE_CAPTURE before bulk read
struct CaptureTiming {
    finger_debounce_ms: u64,
    settle_ms: u64,
    hold_ms: u64,
}

fn capture_timing() -> CaptureTiming {
    CaptureTiming {
        finger_debounce_ms: env_delay_ms(ENV_FINGER_DEBOUNCE_MS, DEFAULT_FINGER_DEBOUNCE_MS),
        settle_ms: env_delay_ms(ENV_CAPTURE_SETTLE_MS, DEFAULT_CAPTURE_SETTLE_MS),
        hold_ms: env_delay_ms(ENV_CAPTURE_HOLD_MS, DEFAULT_CAPTURE_HOLD_MS),
    }
}

fn env_delay_ms(name: &str, default_ms: u64) -> u64 {
    match std::env::var(name) {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .map(|ms| ms.min(MAX_CAPTURE_DELAY_MS))
            .unwrap_or(default_ms),
        Err(_) => default_ms,
    }
}

fn duration_to_timeout_ms(duration: Duration) -> u32 {
    duration.as_millis().min(u128::from(u32::MAX)) as u32
}

/// Wait for a finger-on event that remains stable for `debounce_ms`.
///
/// A stable touch means no `FINGER_OFF` IRQ arrives during the debounce window.
fn wait_for_stable_finger_on(dev: &FpDevice, timeout_ms: u32, debounce_ms: u64) -> Result<()> {
    if debounce_ms == 0 {
        return wait_for_finger(dev, timeout_ms);
    }

    let timeout = if timeout_ms == 0 {
        u32::MAX
    } else {
        timeout_ms
    };
    let deadline = Instant::now() + Duration::from_millis(timeout as u64);

    loop {
        let remaining_total = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining_total.is_zero() {
            return Err(FpError::Timeout);
        }

        // Wait for the next finger-on with the remaining budget.
        wait_for_finger(dev, duration_to_timeout_ms(remaining_total))?;

        let stable_until = Instant::now() + Duration::from_millis(debounce_ms);
        debug_log!(
            "[wait_for_stable_finger_on] debouncing contact for {}ms",
            debounce_ms
        );

        let mut lost_contact = false;
        while Instant::now() < stable_until {
            let until_stable = stable_until
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            let remaining_total = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            let wait_for = until_stable.min(remaining_total);

            if wait_for.is_zero() {
                return Err(FpError::Timeout);
            }

            let mut buf = [0u8; IRQ_LENGTH];
            match dev.handle.read_interrupt(EP_INTR, &mut buf, wait_for) {
                Ok(n) if n >= 2 => {
                    let irq_type = u16::from_be_bytes([buf[0], buf[1]]);
                    debug_log!(
                        "[wait_for_stable_finger_on] got IRQ during debounce: 0x{:04x}",
                        irq_type
                    );
                    if irq_type == IRQDATA_FINGER_OFF {
                        lost_contact = true;
                        break;
                    }
                    // Ignore other IRQs.
                }
                Ok(_) => {
                    // Short read — ignore.
                }
                Err(rusb::Error::Timeout) => {
                    // No IRQ in this interval; continue waiting for stability window.
                }
                Err(e) => {
                    return Err(FpError::UsbIo(e));
                }
            }
        }

        if !lost_contact {
            return Ok(());
        }

        debug_log!("[wait_for_stable_finger_on] contact bounced, retrying finger-on wait");
    }
}

// ── Image decryption ───────────────────────────────────────────────

/// Encryption detection threshold (from libfprint).
///
/// If the variance of the first two image lines exceeds this,
/// the image data is considered encrypted.
const ENC_THRESHOLD: i32 = 5000;

/// Block-info flags from the image header.
mod block_flags {
    pub const CHANGE_KEY: u8 = 0x80;
    pub const NO_KEY_UPDATE: u8 = 0x04;
    pub const ENCRYPTED: u8 = 0x02;
    pub const NOT_PRESENT: u8 = 0x01;
}

/// Parsed image header (used only for decryption decisions).
struct ImageHeader {
    num_lines: u16,
    key_number: u8,
    block_info: [(u8, u8); 15], // (flags, num_lines) per block
}

fn parse_image_header(raw: &[u8]) -> Result<ImageHeader> {
    if raw.len() < IMAGE_HEADER_LEN {
        return Err(FpError::ImageInvalid(format!(
            "header too short: {} bytes",
            raw.len()
        )));
    }
    let num_lines = u16::from_le_bytes([raw[4], raw[5]]);
    let key_number = raw[6];
    let mut block_info = [(0u8, 0u8); 15];
    for (i, slot) in block_info.iter_mut().enumerate() {
        let offset = 16 + i * 2;
        *slot = (raw[offset], raw[offset + 1]);
    }
    Ok(ImageHeader {
        num_lines,
        key_number,
        block_info,
    })
}

/// LFSR key update (from libfprint).
///
/// Taps at bit positions 1, 3, 4, 7, 11, 13, 20, 23, 26, 29, 32.
fn update_key(key: u32) -> u32 {
    let mut bit: u32 = key & 0x9248144d;
    bit ^= bit << 16;
    bit ^= bit << 8;
    bit ^= bit << 4;
    bit ^= bit << 2;
    bit ^= bit << 1;
    (bit & 0x80000000) | (key >> 1)
}

/// Decrypt data in-place using the LFSR stream cipher.
///
/// Matches libfprint's `do_decode` exactly: forward iteration,
/// each plaintext byte = next ciphertext byte XOR key-derived byte.
fn do_decode(data: &mut [u8], mut key: u32) -> u32 {
    let len = data.len();
    if len == 0 {
        return key;
    }
    for i in 0..len - 1 {
        let xorbyte: u8 = (((key >> 4) & 1) as u8)
            | ((((key >> 8) & 1) as u8) << 1)
            | ((((key >> 11) & 1) as u8) << 2)
            | ((((key >> 14) & 1) as u8) << 3)
            | ((((key >> 18) & 1) as u8) << 4)
            | ((((key >> 21) & 1) as u8) << 5)
            | ((((key >> 24) & 1) as u8) << 6)
            | ((((key >> 29) & 1) as u8) << 7);
        key = update_key(key);
        data[i] = data[i + 1] ^ xorbyte;
    }
    data[len - 1] = 0;
    update_key(key)
}

/// Compute a rough variance metric over the first two image lines.
///
/// If the result exceeds [`ENC_THRESHOLD`], the image is encrypted.
fn calc_dev2(pixels: &[u8], header: &ImageHeader) -> i32 {
    let mut lines: [Option<&[u8]>; 2] = [None, None];
    let mut row = 0usize;
    let mut found = 0;

    for i in 0..15 {
        let (flags, count) = header.block_info[i];
        if count == 0 {
            break;
        }
        if flags & block_flags::NOT_PRESENT != 0 {
            continue;
        }
        for _ in 0..count {
            if found < 2 {
                let start = row * IMAGE_WIDTH;
                let end = start + IMAGE_WIDTH;
                if end <= pixels.len() {
                    lines[found] = Some(&pixels[start..end]);
                    found += 1;
                }
            }
            row += 1;
        }
        if found >= 2 {
            break;
        }
    }

    let (b0, b1) = match (lines[0], lines[1]) {
        (Some(a), Some(b)) => (a, b),
        _ => return 0,
    };

    let mut mean: i32 = 0;
    for i in 0..IMAGE_WIDTH {
        mean += b0[i] as i32 + b1[i] as i32;
    }
    mean /= IMAGE_WIDTH as i32;

    let mut res: i32 = 0;
    for i in 0..IMAGE_WIDTH {
        let dev = b0[i] as i32 + b1[i] as i32 - mean;
        res += dev * dev;
    }
    res / IMAGE_WIDTH as i32
}

/// Read the scramble key from the device and decrypt the image data
/// in `raw` if it is encrypted.
///
/// This implements libfprint's `IMAGING_SEND_INDEX → IMAGING_READ_KEY
/// → IMAGING_DECODE` state machine in a synchronous, single-pass
/// fashion.
fn decrypt_image_data(dev: &FpDevice, raw: &mut [u8]) -> Result<()> {
    let header = parse_image_header(raw)?;
    let num_lines = header.num_lines as usize;
    let pixel_start = IMAGE_HEADER_LEN;

    if raw.len() < pixel_start + num_lines * IMAGE_WIDTH {
        return Err(FpError::ImageInvalid("frame too short for decrypt".into()));
    }

    let pixels = &raw[pixel_start..pixel_start + num_lines * IMAGE_WIDTH];
    let dev2 = calc_dev2(pixels, &header);
    debug_log!(
        "[decrypt] variance={}, threshold={}, encrypted={}",
        dev2,
        ENC_THRESHOLD,
        dev2 >= ENC_THRESHOLD
    );

    if dev2 < ENC_THRESHOLD {
        // Image is not encrypted — nothing to do.
        return Ok(());
    }

    // ── Read the encryption key from the device ────────────────
    //
    // libfprint's protocol:
    //   1. Generate a random seed (enc_seed).
    //   2. Write [key_number, seed_le[0..4]] → REG_SCRAMBLE_DATA_INDEX
    //   3. Read 4 bytes ← REG_SCRAMBLE_DATA_KEY
    //   4. key = read_bytes_le ^ enc_seed
    let mut key_number = header.key_number;
    let enc_seed: u32 = rand_seed();
    let key = read_scramble_key(dev, key_number, enc_seed)?;
    debug_log!(
        "[decrypt] key_number={:#04x}, seed={:#010x}, key={:#010x}",
        key_number,
        enc_seed,
        key
    );

    // ── Decrypt block by block ─────────────────────────────────
    let pixels = &mut raw[pixel_start..pixel_start + num_lines * IMAGE_WIDTH];
    let mut current_key = key;
    let mut row = 0usize;

    for i in 0..15 {
        let (flags, count) = header.block_info[i];
        let count = count as usize;
        if count == 0 {
            break;
        }

        debug_log!(
            "[decrypt] block {}: flags={:#04x}, lines={}, row={}",
            i,
            flags,
            count,
            row
        );

        // libfprint: if CHANGE_KEY, re-read the key with a new seed.
        if flags & block_flags::CHANGE_KEY != 0 {
            key_number = key_number.wrapping_add(1);
            let new_seed = rand_seed();
            current_key = read_scramble_key(dev, key_number, new_seed)?;
            debug_log!(
                "[decrypt] CHANGE_KEY → key_number={:#04x}, new_key={:#010x}",
                key_number,
                current_key
            );
        }

        match flags & (block_flags::NO_KEY_UPDATE | block_flags::ENCRYPTED) {
            block_flags::ENCRYPTED => {
                let start = row * IMAGE_WIDTH;
                let end = start + count * IMAGE_WIDTH;
                if end <= pixels.len() {
                    current_key = do_decode(&mut pixels[start..end], current_key);
                }
            }
            0 => {
                // Unencrypted block — advance key state without decrypting.
                for _ in 0..IMAGE_WIDTH * count {
                    current_key = update_key(current_key);
                }
            }
            _ => {
                // NO_KEY_UPDATE (with or without ENCRYPTED) — skip key update.
                // The data is either unencrypted or encrypted with a static key
                // that doesn't advance.  For now, just skip.
            }
        }

        if flags & block_flags::NOT_PRESENT == 0 {
            row += count;
        }
    }

    Ok(())
}

/// Read a 4-byte scramble key from the device.
///
/// Writes `[key_number, seed_le[0..4]]` to `REG_SCRAMBLE_DATA_INDEX`,
/// reads 4 bytes from `REG_SCRAMBLE_DATA_KEY`, XORs with seed.
fn read_scramble_key(dev: &FpDevice, key_number: u8, seed: u32) -> Result<u32> {
    let seed_bytes = seed.to_le_bytes();
    let buf = [
        key_number,
        seed_bytes[0],
        seed_bytes[1],
        seed_bytes[2],
        seed_bytes[3],
    ];
    write_regs(dev, REG_SCRAMBLE_DATA_INDEX, &buf)?;

    let key_bytes = read_reg(dev, REG_SCRAMBLE_DATA_KEY, 4)?;
    if key_bytes.len() < 4 {
        return Err(FpError::UsbIo(rusb::Error::Other));
    }
    let raw_key = u32::from_le_bytes([key_bytes[0], key_bytes[1], key_bytes[2], key_bytes[3]]);
    Ok(raw_key ^ seed)
}

/// Generate a pseudo-random 32-bit seed.
///
/// We don't need cryptographic randomness here — the seed is used to
/// mask the key read from the device, not for security.  The device
/// XORs the key with the seed before returning it, and we XOR it
/// back.  Any non-repeating value works fine.
fn rand_seed() -> u32 {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Mix nanoseconds + seconds for reasonable entropy.
    (t.subsec_nanos() ^ (t.as_secs() as u32)).wrapping_mul(2654435761)
}

/// Run the device initialisation state-machine.
///
/// Pseudo-code (from libfprint):
/// ```text
///   hwstat = read(REG_HWSTAT)
///   if (hwstat & 0x84) == 0x84: reboot_power()
///   if (hwstat & 0x80) == 0:    set_hwstat(hwstat | 0x80)  // power down
///   powerup()                                               // clear bit 7
///   await interrupt(IRQDATA_SCANPWR_ON)                     // 0x56aa
/// ```
fn init_device(dev: &mut FpDevice) -> Result<()> {
    let hwstat = read_hwstat(dev)?;

    // If bits 7 and 2 are both set, the device is in a confused state.
    // Reboot its power by toggling hwstat until bit 1 appears.
    if (hwstat & 0x84) == 0x84 {
        reboot_power(dev)?;
    }

    // Ensure the device is in low-power mode (bit 7 set) before we
    // attempt to power it up cleanly.
    let hwstat = read_hwstat(dev)?;
    if (hwstat & 0x80) == 0 {
        write_reg(dev, REG_HWSTAT, hwstat | 0x80)?;
    }

    // Power the device up: clear bit 7 of hwstat.
    powerup(dev)?;

    // Wait for the 0x56aa interrupt that signals scan power is ready.
    // libfprint uses a 300ms timeout with up to 3 retries.
    for attempt in 0..3 {
        match wait_for_irq(dev, IRQDATA_SCANPWR_ON, 1000) {
            Ok(()) => {
                // Read device info (firmware version) — some devices
                // need this register read to fully complete init.
                let _ = read_reg(dev, REG_DEVICE_INFO, 16);
                return Ok(());
            }
            Err(FpError::Timeout) if attempt < 2 => {
                // Retry the whole init from the top.
                let hwstat = read_hwstat(dev)?;
                if (hwstat & 0x80) == 0 {
                    write_reg(dev, REG_HWSTAT, hwstat | 0x80)?;
                }
                powerup(dev)?;
            }
            Err(e) => return Err(e),
        }
    }

    Err(FpError::Timeout)
}

/// Read the hardware-status register (1 byte).
fn read_hwstat(dev: &FpDevice) -> Result<u8> {
    let data = read_reg(dev, REG_HWSTAT, 1)?;
    Ok(data.first().copied().unwrap_or(0))
}

/// Reboot-power sub-state-machine.
///
/// Masks off the 4 high hwstat bits, then polls until bit 1 appears.
/// Gives up after 100 tries with 10ms pauses.
fn reboot_power(dev: &mut FpDevice) -> Result<()> {
    let hwstat = read_hwstat(dev)?;
    write_reg(dev, REG_HWSTAT, hwstat & 0x0F)?;

    for _ in 0..100 {
        let hwstat = read_hwstat(dev)?;
        if hwstat & 0x01 != 0 {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    Err(FpError::UsbIo(rusb::Error::Other))
}

/// Power-up sub-state-machine.
///
/// Clears bit 7 of hwstat, polls until it stays clear.
/// For DP_URU4000B (our device), no challenge/response auth is needed.
fn powerup(dev: &mut FpDevice) -> Result<()> {
    let hwstat = read_hwstat(dev)?;
    let target = hwstat & 0x0F;

    for _ in 0..100 {
        write_reg(dev, REG_HWSTAT, target)?;
        let current = read_hwstat(dev)?;
        if (current & 0x80) == 0 {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    Err(FpError::UsbIo(rusb::Error::Other))
}

/// Wait for a specific interrupt type on EP_INTR.
fn wait_for_irq(dev: &FpDevice, expected: u16, timeout_ms: u32) -> Result<()> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);

        if remaining.is_zero() {
            return Err(FpError::Timeout);
        }

        let mut buf = [0u8; IRQ_LENGTH];
        match dev.handle.read_interrupt(EP_INTR, &mut buf, remaining) {
            Ok(n) if n >= 2 => {
                let irq_type = u16::from_be_bytes([buf[0], buf[1]]);
                if irq_type == expected {
                    return Ok(());
                }
                // Not our interrupt — keep waiting.
            }
            Ok(_) => {
                // Short read — ignore.
            }
            Err(rusb::Error::Timeout) => {
                return Err(FpError::Timeout);
            }
            Err(e) => {
                return Err(FpError::UsbIo(e));
            }
        }
    }
}

/// Wait for a finger to be placed on the sensor.
///
/// Sets the device to `MODE_AWAIT_FINGER_ON` and polls the interrupt
/// endpoint for `IRQDATA_FINGER_ON`.
fn wait_for_finger(dev: &FpDevice, timeout_ms: u32) -> Result<()> {
    debug_log!(
        "[wait_for_finger] setting MODE_AWAIT_FINGER_ON (0x{:02x})",
        MODE_AWAIT_FINGER_ON
    );
    write_reg(dev, REG_MODE, MODE_AWAIT_FINGER_ON)?;

    let timeout = if timeout_ms == 0 {
        u32::MAX
    } else {
        timeout_ms
    };
    let deadline = Instant::now() + Duration::from_millis(timeout as u64);

    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);

        if remaining.is_zero() {
            debug_log!("[wait_for_finger] TIMEOUT");
            return Err(FpError::Timeout);
        }

        let mut buf = [0u8; IRQ_LENGTH];
        match dev.handle.read_interrupt(EP_INTR, &mut buf, remaining) {
            Ok(n) if n >= 2 => {
                let irq_type = u16::from_be_bytes([buf[0], buf[1]]);
                debug_log!(
                    "[wait_for_finger] got IRQ: 0x{:04x} ({} bytes)",
                    irq_type,
                    n
                );
                if irq_type == IRQDATA_FINGER_ON {
                    return Ok(());
                }
                // Ignore other interrupt types.
            }
            Ok(n) => {
                debug_log!("[wait_for_finger] short IRQ read: {} bytes", n);
            }
            Err(rusb::Error::Timeout) => {
                debug_log!("[wait_for_finger] TIMEOUT (usb)");
                return Err(FpError::Timeout);
            }
            Err(e) => {
                debug_log!("[wait_for_finger] USB error: {}", e);
                return Err(FpError::UsbIo(e));
            }
        }
    }
}

/// Wait for the finger to be removed from the sensor.
///
/// Sets the device to `MODE_AWAIT_FINGER_OFF` and polls the interrupt
/// endpoint for `IRQDATA_FINGER_OFF`.  This must be called after
/// a successful capture so the device can cleanly detect the next
/// finger-on event.
fn wait_for_finger_off(dev: &FpDevice, timeout_ms: u32) -> Result<()> {
    debug_log!(
        "[wait_for_finger_off] setting MODE_AWAIT_FINGER_OFF (0x{:02x})",
        MODE_AWAIT_FINGER_OFF
    );
    write_reg(dev, REG_MODE, MODE_AWAIT_FINGER_OFF)?;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);

        if remaining.is_zero() {
            debug_log!("[wait_for_finger_off] TIMEOUT");
            return Err(FpError::Timeout);
        }

        let mut buf = [0u8; IRQ_LENGTH];
        match dev.handle.read_interrupt(EP_INTR, &mut buf, remaining) {
            Ok(n) if n >= 2 => {
                let irq_type = u16::from_be_bytes([buf[0], buf[1]]);
                debug_log!(
                    "[wait_for_finger_off] got IRQ: 0x{:04x} ({} bytes)",
                    irq_type,
                    n
                );
                if irq_type == IRQDATA_FINGER_OFF {
                    return Ok(());
                }
                // Ignore other interrupt types.
            }
            Ok(n) => {
                debug_log!("[wait_for_finger_off] short IRQ read: {} bytes", n);
            }
            Err(rusb::Error::Timeout) => {
                debug_log!("[wait_for_finger_off] TIMEOUT (usb)");
                return Err(FpError::Timeout);
            }
            Err(e) => {
                debug_log!("[wait_for_finger_off] USB error: {}", e);
                return Err(FpError::UsbIo(e));
            }
        }
    }
}

/// Attempt [`bulk_read_image`] up to `MAX_CAPTURE_RETRIES` times.
///
/// The U.are.U 4500 sometimes sends a zero-length packet (ZLP) on
/// the first bulk read after `MODE_CAPTURE`, particularly on
/// subsequent captures.  libfprint handles this by re-submitting
/// the bulk transfer from the `IMAGING_CAPTURE` state without
/// re-writing the mode register.  We emulate this by retrying.
const MAX_CAPTURE_RETRIES: u32 = 3;

fn bulk_read_image_with_retry(dev: &FpDevice) -> Result<Vec<u8>> {
    for attempt in 1..=MAX_CAPTURE_RETRIES {
        debug_log!(
            "[bulk_read_retry] attempt {}/{}",
            attempt,
            MAX_CAPTURE_RETRIES
        );
        match bulk_read_image(dev) {
            Ok(buf) => return Ok(buf),
            Err(FpError::ImageInvalid(ref msg)) if attempt < MAX_CAPTURE_RETRIES => {
                debug_log!(
                    "[bulk_read_retry] short/bad read on attempt {}: {}  — retrying",
                    attempt,
                    msg
                );
                // Do NOT re-write MODE_CAPTURE.  The device is
                // still in capture mode and will send data on
                // the next bulk read.
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    // Unreachable unless MAX_CAPTURE_RETRIES is 0.
    Err(FpError::ImageInvalid("capture retries exhausted".into()))
}

/// Read the raw image data from the bulk endpoint.
///
/// Returns the full raw buffer including the 64-byte header.
fn bulk_read_image(dev: &FpDevice) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; DATABLK_RQLEN];
    let mut total_read = 0usize;

    debug_log!(
        "[bulk_read] starting (want up to {} bytes)...",
        DATABLK_RQLEN
    );

    // The device may split the transfer across multiple USB packets.
    // Keep reading until we have enough data or timeout.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut read_count = 0u32;

    while total_read < DATABLK_RQLEN {
        let remaining_time = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);

        if remaining_time.is_zero() {
            debug_log!(
                "[bulk_read] deadline reached after {} reads, {} bytes",
                read_count,
                total_read
            );
            break;
        }

        match dev
            .handle
            .read_bulk(EP_DATA, &mut buf[total_read..], remaining_time)
        {
            Ok(n) => {
                read_count += 1;
                debug_log!(
                    "[bulk_read] read #{}: {} bytes (total: {})",
                    read_count,
                    n,
                    total_read + n
                );
                total_read += n;
                if n == 0 {
                    debug_log!("[bulk_read] ZLP — stopping");
                    break; // ZLP or device done
                }
            }
            Err(rusb::Error::Timeout) => {
                debug_log!(
                    "[bulk_read] timeout after {} reads, {} bytes",
                    read_count,
                    total_read
                );
                break;
            }
            Err(e) => {
                debug_log!("[bulk_read] USB error: {}", e);
                return Err(FpError::UsbIo(e));
            }
        }
    }

    debug_log!(
        "[bulk_read] done: {} total bytes in {} reads",
        total_read,
        read_count
    );

    if total_read < IMAGE_HEADER_LEN {
        return Err(FpError::ImageInvalid(format!(
            "read only {} bytes, need at least {} for header",
            total_read, IMAGE_HEADER_LEN
        )));
    }

    buf.truncate(total_read);
    Ok(buf)
}
