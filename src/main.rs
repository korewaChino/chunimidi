use color_eyre::Result;
use midir::{MidiOutput, MidiOutputConnection};
use std::convert::TryFrom;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, error, info};

const LED_PACKET_FRAMING: u8 = 0xE0;
const LED_PACKET_ESCAPE: u8 = 0xD0;
const LED_BOARDS_TOTAL: usize = 3;
const CHUNI_LED_BOARD_DATA_LENS: [usize; LED_BOARDS_TOTAL] = [53 * 3, 63 * 3, 31 * 3];

#[derive(Debug)]
enum DecodeError {
    Invalid,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug)]
pub enum LedBoard {
    BillboardLeft([Rgb; 53], [Rgb; 3]),
    BillboardRight([Rgb; 60], [Rgb; 3]),
    Slider([Rgb; 31]),
}

#[derive(Debug)]
pub struct LedPacket {
    pub board: u8,
    pub payload: LedBoard,
}

fn bytes_to_rgb_vec(data: &[u8]) -> Vec<Rgb> {
    data.chunks(3)
        .filter(|chunk| chunk.len() == 3)
        .map(|chunk| Rgb {
            r: chunk[1],
            g: chunk[2],
            b: chunk[0],
        })
        .collect()
}

fn reverse_slider_leds(leds: [Rgb; 31]) -> [Rgb; 31] {
    let mut reversed = leds;
    reversed.reverse();
    reversed
}

fn slider_to_drum_pads(leds: [Rgb; 31]) -> [Rgb; 8] {
    let mut drum_pads = [Rgb { r: 0, g: 0, b: 0 }; 8];

    for (pad_idx, pad) in drum_pads.iter_mut().enumerate() {
        let start_led = pad_idx * 4;
        let end_led = (start_led + 4).min(31); // Handle the last group which might have fewer than 4 LEDs

        // Average the RGB values of the 4 LEDs in this group
        let mut total_r = 0u32;
        let mut total_g = 0u32;
        let mut total_b = 0u32;
        let mut count = 0u32;

        for led in &leds[start_led..end_led] {
            total_r += led.r as u32;
            total_g += led.g as u32;
            total_b += led.b as u32;
            count += 1;
        }

        if count > 0 {
            pad.r = (total_r / count) as u8;
            pad.g = (total_g / count) as u8;
            pad.b = (total_b / count) as u8;
        }
    }

    drum_pads
}

fn rgb_to_launchkey_velocity(rgb: Rgb) -> u8 {
    // Launchkey Mini MK3 color palette mapping
    // Based on the official color palette documentation

    // Calculate color distances to find the closest match
    let palette = [
        (0, (0, 0, 0)),        // 0: Black/Off
        (1, (128, 128, 128)),  // 1: Dark Gray
        (2, (192, 192, 192)),  // 2: Light Gray
        (3, (255, 255, 255)),  // 3: White
        (4, (255, 192, 192)),  // 4: Light Pink
        (5, (255, 0, 0)),      // 5: Red
        (6, (192, 0, 0)),      // 6: Dark Red
        (7, (128, 0, 0)),      // 7: Very Dark Red
        (8, (255, 192, 128)),  // 8: Light Orange
        (9, (255, 128, 0)),    // 9: Orange
        (10, (192, 96, 0)),    // 10: Dark Orange
        (11, (128, 64, 0)),    // 11: Brown
        (12, (255, 255, 128)), // 12: Light Yellow
        (13, (255, 255, 0)),   // 13: Yellow
        (14, (192, 192, 0)),   // 14: Dark Yellow
        (15, (128, 128, 0)),   // 15: Olive
        (16, (192, 255, 128)), // 16: Light Green
        (17, (128, 255, 0)),   // 17: Bright Green
        (18, (96, 192, 0)),    // 18: Green
        (19, (64, 128, 0)),    // 19: Dark Green
        (20, (192, 255, 192)), // 20: Very Light Green
        (21, (0, 255, 0)),     // 21: Pure Green
        (22, (0, 192, 0)),     // 22: Medium Green
        (23, (0, 128, 0)),     // 23: Forest Green
        (24, (128, 255, 192)), // 24: Light Mint
        (25, (0, 255, 128)),   // 25: Mint Green
        (26, (0, 192, 96)),    // 26: Teal Green
        (27, (0, 128, 64)),    // 27: Dark Teal
        (28, (128, 255, 255)), // 28: Light Cyan
        (29, (0, 255, 255)),   // 29: Cyan
        (30, (0, 192, 192)),   // 30: Dark Cyan
        (31, (0, 128, 128)),   // 31: Teal
        (32, (128, 192, 255)), // 32: Light Blue
        (33, (0, 128, 255)),   // 33: Sky Blue
        (34, (0, 96, 192)),    // 34: Blue
        (35, (0, 64, 128)),    // 35: Dark Blue
        (36, (128, 128, 255)), // 36: Light Purple
        (37, (0, 0, 255)),     // 37: Pure Blue
        (38, (0, 0, 192)),     // 38: Medium Blue
        (39, (0, 0, 128)),     // 39: Navy Blue
        (40, (192, 128, 255)), // 40: Light Violet
        (41, (128, 0, 255)),   // 41: Purple
        (42, (96, 0, 192)),    // 42: Dark Purple
        (43, (64, 0, 128)),    // 43: Very Dark Purple
        (44, (255, 128, 255)), // 44: Light Magenta
        (45, (255, 0, 255)),   // 45: Magenta
        (46, (192, 0, 192)),   // 46: Dark Magenta
        (47, (128, 0, 128)),   // 47: Purple
        (48, (255, 128, 192)), // 48: Light Pink
        (49, (255, 0, 128)),   // 49: Hot Pink
        (50, (192, 0, 96)),    // 50: Dark Pink
        (51, (128, 0, 64)),    // 51: Maroon
    ];

    let mut best_velocity = 0;
    let mut best_distance = f64::INFINITY;

    for (velocity, (pr, pg, pb)) in palette {
        let distance = ((rgb.r as f64 - pr as f64).powi(2)
            + (rgb.g as f64 - pg as f64).powi(2)
            + (rgb.b as f64 - pb as f64).powi(2))
        .sqrt();

        if distance < best_distance {
            best_distance = distance;
            best_velocity = velocity;
        }
    }

    best_velocity
}

fn send_rgb_to_launchkey(conn: &mut MidiOutputConnection, drum_pads: [Rgb; 8]) -> Result<()> {
    for (pad_idx, rgb) in drum_pads.iter().enumerate() {
        // Use bottom drum pads (notes 112-119, 0x70-0x77)
        let pad_note = 0x70 + pad_idx as u8;

        // Map RGB to Launchkey velocity using color mapping
        let velocity = rgb_to_launchkey_velocity(*rgb);

        debug!(
            "Sending pad {} (note {}) → velocity {} (RGB: {}, {}, {})",
            pad_idx, pad_note, velocity, rgb.r, rgb.g, rgb.b
        );

        // Send Note On message on Channel 1 (0x90) with velocity representing color
        let note_on_msg = [
            0x90,     // Note On, Channel 1
            pad_note, // Note number (drum pad 112-119)
            velocity, // Velocity (maps to specific color)
        ];

        conn.send(&note_on_msg)?;
    }

    Ok(())
}

fn enable_daw_mode(conn: &mut MidiOutputConnection) -> Result<()> {
    info!("Enabling DAW mode...");
    conn.send(&[0x9F, 0x0C, 0x7F])?;
    info!("DAW mode enabled");
    Ok(())
}

fn disable_daw_mode(conn: &mut MidiOutputConnection) -> Result<()> {
    info!("Disabling DAW mode...");
    conn.send(&[0x9F, 0x0C, 0x00])?;
    info!("DAW mode disabled");
    Ok(())
}

fn send_test_colors(conn: &mut MidiOutputConnection) -> Result<()> {
    info!("Blacking out top pads (96-103)...");

    // Black out top pads (notes 96-103)
    for note in 96..=103 {
        let note_off_msg = [
            0x90, // Note On, Channel 1
            note, // Note number (top pads 96-103)
            0,    // Velocity 0 (off/black)
        ];
        conn.send(&note_off_msg)?;
    }

    info!("Sending test colors (all green) to drum pads...");

    // Create test drum pads - all green
    let test_pads = [Rgb { r: 0, g: 255, b: 0 }; 8];

    send_rgb_to_launchkey(conn, test_pads)?;

    info!("Test colors sent!");
    Ok(())
}

fn try_parse_packet(buf: &[u8]) -> Result<(LedPacket, usize), DecodeError> {
    if buf.len() < 2 || buf[0] != LED_PACKET_FRAMING {
        return Err(DecodeError::Invalid);
    }

    let board = buf[1];
    if (board as usize) >= LED_BOARDS_TOTAL {
        return Err(DecodeError::Invalid);
    }

    let expected_data_len = CHUNI_LED_BOARD_DATA_LENS[board as usize];
    let mut decoded = Vec::with_capacity(expected_data_len);
    let mut i = 2;

    while i < buf.len() && decoded.len() < expected_data_len {
        let b = buf[i];
        if b == LED_PACKET_ESCAPE {
            i += 1;
            if i >= buf.len() {
                return Err(DecodeError::Incomplete);
            }
            decoded.push(buf[i].wrapping_add(1));
        } else {
            decoded.push(b);
        }
        i += 1;
    }

    if decoded.len() < expected_data_len {
        return Err(DecodeError::Incomplete);
    }

    let rgb_vec = bytes_to_rgb_vec(&decoded);
    let payload = match board {
        0 => {
            let (main, sides) = rgb_vec.split_at(53);
            LedBoard::BillboardLeft(
                <[Rgb; 53]>::try_from(main).unwrap(),
                <[Rgb; 3]>::try_from(sides).unwrap(),
            )
        }
        1 => {
            let (main, sides) = rgb_vec.split_at(60);
            LedBoard::BillboardRight(
                <[Rgb; 60]>::try_from(main).unwrap(),
                <[Rgb; 3]>::try_from(sides).unwrap(),
            )
        }
        2 => LedBoard::Slider(reverse_slider_leds(
            <[Rgb; 31]>::try_from(&rgb_vec[..]).unwrap(),
        )),
        _ => return Err(DecodeError::Invalid),
    };

    Ok((LedPacket { board, payload }, i))
}

fn setup_signal_handler() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    std::thread::spawn(move || {
        let mut signals = signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
        ])
        .expect("Failed to create signal handler");

        if let Some(sig) = signals.forever().next() {
            info!("Received signal: {}", sig);
            r.store(false, Ordering::SeqCst);
        }
    });

    running
}

fn main() -> Result<()> {
    color_eyre::install()?;

    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Set up signal handler
    let running = setup_signal_handler();

    // Track last sent drum pad state to prevent redundant messages
    let mut last_drum_pads: Option<[Rgb; 8]> = None;

    // Initialize MIDI output
    let midi_output = MidiOutput::new("chunimidi")?;
    let out_ports = midi_output.ports();

    // Find the Launchkey Mk3 at port 16:1
    let launchkey_port = out_ports
        .iter()
        .find(|port| {
            let port_name = midi_output.port_name(port).unwrap_or_default();
            debug!("Found MIDI port: {}", port_name);
            port_name.contains("16:1")
        })
        .ok_or_else(|| color_eyre::eyre::eyre!("Launchkey MK3 not found"))?;
    let mut midi_conn = midi_output
        .connect(launchkey_port, "chunimidi-launchkey")
        .map_err(|e| color_eyre::eyre::eyre!("Failed to connect to MIDI device: {}", e))?;
    info!("Connected to Launchkey MK3");

    // Enable DAW mode (includes programmer mode)
    enable_daw_mode(&mut midi_conn)?;

    // Send test colors
    send_test_colors(&mut midi_conn)?;

    // Enable real LED data processing when socket is available
    let socket_path = "/tmp/chuni.sock"; // Change to your socket path

    // Try to connect to LED socket, but continue without it if not available
    match UnixStream::connect(Path::new(socket_path)) {
        Ok(mut stream) => {
            info!("Connected to LED socket: {}", socket_path);

            let mut buf = vec![0u8; 4096];
            let mut window = Vec::<u8>::new();

            while running.load(Ordering::SeqCst) {
                let n = stream.read(&mut buf)?;
                if n == 0 {
                    break;
                }

                window.extend_from_slice(&buf[..n]);

                while let Some(pos) = window.iter().position(|&b| b == LED_PACKET_FRAMING) {
                    if pos > 0 {
                        window.drain(0..pos); // drop garbage before sync
                    }

                    match try_parse_packet(&window) {
                        Ok((packet, used)) => {
                            // debug!("Decoded LED packet: {:?}", packet);

                            // If it's a slider packet, also show the drum pad conversion
                            if let LedBoard::Slider(slider_leds) = &packet.payload {
                                let drum_pads = slider_to_drum_pads(*slider_leds);
                                // debug!("Drum pads (8 zones): {:?}", drum_pads);

                                // Only send RGB data to Launchkey Mk3 if colors have changed
                                let should_send = match &last_drum_pads {
                                    None => true,                     // First time, always send
                                    Some(last) => *last != drum_pads, // Only send if different
                                };

                                if should_send {
                                    if let Err(e) = send_rgb_to_launchkey(&mut midi_conn, drum_pads)
                                    {
                                        error!("Failed to send MIDI data: {}", e);
                                    } else {
                                        debug!(
                                            "Updated Launchkey colors (changed from previous state)"
                                        );
                                        last_drum_pads = Some(drum_pads);
                                    }
                                } else {
                                    // Uncomment the line below if you want to see when updates are skipped
                                    // debug!("Skipping MIDI update (colors unchanged)");
                                }
                            }

                            window.drain(0..used);
                        }
                        Err(DecodeError::Incomplete) => break, // wait for more data
                        Err(DecodeError::Invalid) => {
                            window.drain(0..1); // skip bad byte
                        }
                    }
                }
            }
        }
        Err(e) => {
            info!(
                "LED socket not available ({}), running in test mode only",
                e
            );
            info!("Press Ctrl+C to exit...");

            // Just wait for signal to exit
            while running.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    // Cleanup: disable DAW mode before exiting
    info!("Shutting down...");
    if let Err(e) = disable_daw_mode(&mut midi_conn) {
        error!("Failed to disable DAW mode: {}", e);
    }

    Ok(())
}
