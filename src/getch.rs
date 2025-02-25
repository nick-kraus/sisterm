
#[cfg(windows)]
use libc::c_int;
#[cfg(windows)]
use winapi::{
    um::consoleapi::{GetConsoleMode, SetConsoleMode},
    um::handleapi::INVALID_HANDLE_VALUE,
    um::processenv::GetStdHandle,
    um::winbase::STD_INPUT_HANDLE,
    um::wincon::ENABLE_ECHO_INPUT,
};

#[cfg(windows)]
extern "C" {
    fn _getch() -> c_int;
}

#[cfg(not(windows))]
use nix::sys::termios;
#[cfg(not(windows))]
use std::io::Read;
#[cfg(not(windows))]
use std::cell::RefCell;


#[cfg(windows)]
pub struct Getch {}

#[cfg(not(windows))]
pub struct Getch {
    orig_term: termios::Termios,
    leftover:  RefCell<Option<u8>>,
}


/// A key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// Null byte.
    Null,
    /// Backspace.
    Backspace,
    /// Delete key.
    Delete,
    /// Esc key.
    Esc,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Right arrow.
    Right,
    /// Left arrow.
    Left,
    /// End key.
    End,
    /// Home key.
    Home,
    /// Backward Tab key.
    BackTab,
    /// Insert key.
    Insert,
    /// Page Up key.
    PageUp,
    /// Page Down key.
    PageDown,
    /// Function keys.
    ///
    /// Only function keys 1 through 12 are supported.
    F(u8),
    /// Normal character.
    Char(char),
    /// Alt modified character.
    Alt(char),
    /// Ctrl modified character.
    ///
    /// Note that certain keys may not be modifiable with `ctrl`, due to limitations of terminals.
    Ctrl(char),
    /// Other key.
    Other(Vec<u8>),
}

impl Getch {
    #[cfg(windows)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {}
    }
    #[cfg(not(windows))]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        // Quering original as a separate, since `Termios` does not implement copy
        let orig_term       = termios::tcgetattr(0).unwrap();
        let mut raw_termios = termios::tcgetattr(0).unwrap();

        // Unset canonical mode, so we get characters immediately
        raw_termios.local_flags.remove(termios::LocalFlags::ICANON);
        // Don't generate signals on Ctrl-C and friends
        raw_termios.local_flags.remove(termios::LocalFlags::ISIG);
        // Disable local echo
        raw_termios.local_flags.remove(termios::LocalFlags::ECHO);

        termios::tcsetattr(0, termios::SetArg::TCSADRAIN, &raw_termios).unwrap();

        Self {
            orig_term,
            leftover: RefCell::new(None),
        }
    }

    #[cfg(windows)]
    pub fn getch(&self) -> Result<Key, std::io::Error> {
        use winapi::um::winuser::{GetKeyState, VK_MENU};

        unsafe {
            match _getch() {
                0x00 | 0xE0 => {
                    match _getch() {
                        0x48 => Ok(Key::Up),
                        0x50 => Ok(Key::Down),
                        0x4D => Ok(Key::Right),
                        0x4B => Ok(Key::Left),
                        0x47 => Ok(Key::Home),
                        0x4F => Ok(Key::End),
                        0x49 => Ok(Key::PageUp),
                        0x51 => Ok(Key::PageDown),
                        0x52 => Ok(Key::Insert),
                        0x53 => Ok(Key::Delete),
                        v @ 0x3B..=0x44 => {
                            Ok(Key::F((v - 0x3B + 1) as u8))
                        },
                        0x85 => Ok(Key::F(11)),
                        0x86 => Ok(Key::F(12)),
                        0x73 => Ok(Key::Other(vec![ b'\x1B', b'[', b'1', b';', b'5', b'D' ])),  // Ctrl+Left
                        0x74 => Ok(Key::Other(vec![ b'\x1B', b'[', b'1', b';', b'5', b'C' ])),  // Ctrl+Right
                        0x8D => Ok(Key::Other(vec![ b'\x1B', b'[', b'1', b';', b'5', b'A' ])),  // Ctrl+Up
                        0x91 => Ok(Key::Other(vec![ b'\x1B', b'[', b'1', b';', b'5', b'B' ])),  // Ctrl+Down
                        c    => Ok(Key::Other(vec![ b'\x1B', c as u8 ])),
                    }
                },
                0x08        => Ok(Key::Backspace),
                0x1B        => Ok(Key::Esc),
                0x7F        => Ok(Key::Delete),
                0x0A | 0x0D => Ok(Key::Char('\r')),
                0x09        => Ok(Key::Char('\t')),
                c @ 0x01..=0x1A => {
                    Ok(Key::Ctrl((c as u8 + b'a' - 1) as char))
                },
                c => {
                    Ok(
                        match parse_utf8_char(c as u8)? {
                            Ok(ch)   => {
                                if GetKeyState(VK_MENU) < 0 {
                                    Key::Alt(ch)
                                } else {
                                    Key::Char(ch)
                                }
                            },
                            Err(vec) => Key::Other(vec),
                        }
                    )
                }
            }
        }
    }
    #[cfg(not(windows))]
    #[allow(clippy::unused_io_amount)]
    pub fn getch(&self) -> Result<Key, std::io::Error> {
        let source = &mut std::io::stdin();
        let mut buf: [u8; 2] = [0; 2];

        if self.leftover.borrow().is_some() {
            // we have a leftover byte, use it
            let c = self.leftover.borrow().unwrap();
            self.leftover.replace(None);
            return parse_key(c, &mut source.bytes());
        }

        match source.read(&mut buf) {
            Ok(0) => Ok(Key::Null),
            Ok(1) => {
                match buf[0] {
                    b'\x1B' => Ok(Key::Esc),
                    c => parse_key(c, &mut source.bytes()),
                }
            },
            Ok(2) => {
                let option_iter = &mut Some(buf[1]).into_iter();
                let result = {
                    let mut iter = option_iter.map(Ok).chain(source.bytes());
                    parse_key(buf[0], &mut iter)
                };
                // If the option_iter wasn't consumed, keep the byte for later.
                self.leftover.replace(option_iter.next());
                result
            },
            Ok(_) => unreachable!(),
            Err(e) => Err(e),
        }
    }
}

/// Enable local echo
pub fn enable_echo_input() {
    #[cfg(windows)]
    unsafe {
        let input_handle = GetStdHandle(STD_INPUT_HANDLE);
        let mut console_mode: u32 = 0;

        if input_handle == INVALID_HANDLE_VALUE {
            return;
        }

        if GetConsoleMode(input_handle, &mut console_mode) != 0 {
            SetConsoleMode(input_handle, console_mode | ENABLE_ECHO_INPUT);
        }
    }

    #[cfg(not(windows))]
    {
        let mut raw_termios = termios::tcgetattr(0).unwrap();
        raw_termios.local_flags.insert(termios::LocalFlags::ECHO);
        termios::tcsetattr(0, termios::SetArg::TCSADRAIN, &raw_termios).unwrap();
    }
}

/// Disable local echo
pub fn disable_echo_input() {
    #[cfg(windows)]
    unsafe {
        let input_handle = GetStdHandle(STD_INPUT_HANDLE);
        let mut console_mode: u32 = 0;

        if input_handle == INVALID_HANDLE_VALUE {
            return;
        }

        if GetConsoleMode(input_handle, &mut console_mode) != 0 {
            SetConsoleMode(input_handle, console_mode & !ENABLE_ECHO_INPUT);
        }
    }

    #[cfg(not(windows))]
    {
        let mut raw_termios = termios::tcgetattr(0).unwrap();
        raw_termios.local_flags.remove(termios::LocalFlags::ECHO);
        termios::tcsetattr(0, termios::SetArg::TCSADRAIN, &raw_termios).unwrap();
    }
}

/// Parse an Event from `item` and possibly subsequent bytes through `iter`.
#[cfg(not(windows))]
fn parse_key<I>(item: u8, iter: &mut I) -> Result<Key, std::io::Error>
where
    I: Iterator<Item = Result<u8, std::io::Error>>
{
        match item {
            b'\x1B' => {
                Ok(match iter.next() {
                    Some(Ok(b'[')) => parse_csi(iter)?,
                    Some(Ok(b'O')) => {
                        match iter.next() {
                            // F1-F4
                            Some(Ok(val @ b'P'..=b'S')) => Key::F(1 + val - b'P'),
                            Some(Ok(val)) => Key::Other(vec![ b'\x1B', b'O', val ]),
                            _ => Key::Other(vec![ b'\x1B', b'O' ]),
                        }
                    },
                    Some(Ok(c)) => {
                        match parse_utf8_char(c, iter)? {
                            Ok(ch)   => Key::Alt(ch),
                            Err(vec) => Key::Other(vec),
                        }
                    },
                    Some(Err(e)) => return Err(e),
                    None => Key::Esc,
                })
            },
            b'\n' | b'\r'         => Ok(Key::Char('\r')),
            b'\t'                 => Ok(Key::Char('\t')),
            b'\x08'               => Ok(Key::Backspace),
            b'\x7F'               => Ok(Key::Delete),
            c @ b'\x01'..=b'\x1A' => Ok(Key::Ctrl((c as u8 - 0x1  + b'a') as char)),
            c @ b'\x1C'..=b'\x1F' => Ok(Key::Ctrl((c as u8 - 0x1C + b'4') as char)),
            b'\0'                 => Ok(Key::Null),
            c => {
                Ok(
                    match parse_utf8_char(c, iter)? {
                        Ok(ch)   => Key::Char(ch),
                        Err(vec) => Key::Other(vec),
                    }
                )
            }
        }
}

/// Parses a CSI sequence, just after reading ^[
///
/// Returns None if an unrecognized sequence is found.
#[cfg(not(windows))]
fn parse_csi<I>(iter: &mut I) -> Result<Key, std::io::Error>
where
     I: Iterator<Item = Result<u8, std::io::Error>>
{
    Ok(match iter.next() {
        Some(Ok(b'[')) => {
            match iter.next() {
                Some(Ok(val @ b'A'..=b'E')) => Key::F(1 + val - b'A'),
                Some(Ok(val)) => Key::Other(vec![ b'\x1B', b'[', b'[', val ]),
                _ => Key::Other(vec![ b'\x1B', b'[', b'[' ]),
            }
        },
        Some(Ok(b'A')) => Key::Up,
        Some(Ok(b'B')) => Key::Down,
        Some(Ok(b'C')) => Key::Right,
        Some(Ok(b'D')) => Key::Left,
        Some(Ok(b'F')) => Key::End,
        Some(Ok(b'H')) => Key::Home,
        Some(Ok(b'Z')) => Key::BackTab,
        Some(Ok(c @ b'0'..=b'9')) => {
            // Numbered escape code.
            let mut buf = vec![c];
            let mut c = iter.next().unwrap().unwrap();
            // The final byte of a CSI sequence can be in the range 64-126, so
            // let's keep reading anything else.
            while !(64..=126).contains(&c) {
                buf.push(c);
                c = iter.next().unwrap().unwrap();
            }
            match c {
                // Special key code.
                b'~' => {
                    let str_buf = std::str::from_utf8(&buf).unwrap();

                    // This CSI sequence can be a list of semicolon-separated
                    // numbers.
                    let nums: Vec<u8> = str_buf.split(';').map(|n| n.parse().unwrap()).collect();

                    if nums.is_empty() || nums.len() > 1 {
                        let mut keys = vec![ b'\x1B', b'['];
                        keys.append(&mut buf);
                        return Ok(Key::Other(keys));
                    }

                    match nums[0] {
                        1 | 7 => Key::Home,
                        2     => Key::Insert,
                        3     => Key::Delete,
                        4 | 8 => Key::End,
                        5     => Key::PageUp,
                        6     => Key::PageDown,
                        v @ 11..=15 => Key::F(v - 10),
                        v @ 17..=21 => Key::F(v - 11),
                        v @ 23..=24 => Key::F(v - 12),
                        _ => {
                            let mut keys = vec![ b'\x1B', b'[' ];
                            keys.append(&mut buf);
                            keys.push(nums[0]);
                            return Ok(Key::Other(keys));
                        },
                    }
                },
                _ => {
                    let mut keys = vec![ b'\x1B', b'[' ];
                    keys.append(&mut buf);
                    keys.push(c);
                    return Ok(Key::Other(keys));
                },
            }
        },
        Some(Ok(c)) => Key::Other(vec![ b'\x1B', b'[', c ]),
        _ => Key::Other(vec![ b'\x1B', b'[' ]),
    })
}

/// Parse `c` as either a single byte ASCII char or a variable size UTF-8 char.
#[cfg(windows)]
fn parse_utf8_char(c: u8) -> Result<Result<char, Vec<u8>>, std::io::Error> {
    if c.is_ascii() {
        Ok(Ok(c as char))
    } else {
        let bytes = &mut Vec::new();
        bytes.push(c);

        loop {
            let next = unsafe { _getch() } as u8;
            bytes.push(next);
            if let Ok(st) = std::str::from_utf8(bytes) {
                return Ok(Ok(st.chars().next().unwrap()));
            }
            if bytes.len() >= 4 {
                return Ok(Err(bytes.to_vec()));
            }
        }
    }
}

/// Parse `c` as either a single byte ASCII char or a variable size UTF-8 char.
#[cfg(not(windows))]
fn parse_utf8_char<I>(c: u8, iter: &mut I) -> Result<Result<char, Vec<u8>>, std::io::Error>
where
     I: Iterator<Item = Result<u8, std::io::Error>>
{
    if c.is_ascii() {
        Ok(Ok(c as char))
    } else {
        let bytes = &mut Vec::new();
        bytes.push(c);

        loop {
            match iter.next() {
                Some(Ok(next)) => {
                    bytes.push(next);
                    if let Ok(st) = std::str::from_utf8(bytes) {
                        return Ok(Ok(st.chars().next().unwrap()));
                    }
                    if bytes.len() >= 4 {
                        return Ok(Err(bytes.to_vec()));
                    }
                },
                _ => return Ok(Err(bytes.to_vec())),
            }
        }
    }
}

impl Drop for Getch {
    #[cfg(windows)]
    fn drop(&mut self) {}

    #[cfg(not(windows))]
    fn drop(&mut self) {
        termios::tcsetattr(0, termios::SetArg::TCSADRAIN, &self.orig_term).unwrap();
    }

}
