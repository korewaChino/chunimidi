use color_eyre::Result;
use std::convert::TryFrom;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;

const LED_PACKET_FRAMING: u8 = 0xE0;
const LED_PACKET_ESCAPE: u8 = 0xD0;
const LED_BOARDS_TOTAL: usize = 3;
const CHUNI_LED_BOARD_DATA_LENS: [usize; LED_BOARDS_TOTAL] = [53 * 3, 63 * 3, 31 * 3];

#[derive(Debug)]
enum DecodeError {
    Invalid,
    Incomplete,
}

#[derive(Debug, Clone, Copy)]
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

        for led_idx in start_led..end_led {
            total_r += leds[led_idx].r as u32;
            total_g += leds[led_idx].g as u32;
            total_b += leds[led_idx].b as u32;
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

fn main() -> Result<()> {
    color_eyre::install()?;

    let midi_device

    let socket_path = "/tmp/chuni.sock"; // Change to your socket path
    let mut stream = UnixStream::connect(Path::new(socket_path))?;
    println!("[+] Connected to LED socket: {}", socket_path);

    let mut buf = vec![0u8; 4096];
    let mut window = Vec::<u8>::new();

    loop {
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
                    println!("[*] Decoded LED packet: {:?}", packet);

                    // If it's a slider packet, also show the drum pad conversion
                    if let LedBoard::Slider(slider_leds) = &packet.payload {
                        let drum_pads = slider_to_drum_pads(*slider_leds);
                        println!("[*] Drum pads (8 zones): {:?}", drum_pads);
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

    Ok(())
}
