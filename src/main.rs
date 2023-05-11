use rand::{
    distributions::{Alphanumeric, DistString},
    Rng,
};
use signal_hook::{consts::SIGWINCH, iterator::Signals};
use std::{
    collections::HashMap,
    error::Error,
    io::{stdout, Read, StdoutLock, Write},
    sync::mpsc::{self, Sender},
    thread,
};
use termion::{
    async_stdin, clear, color, cursor,
    event::{parse_event, Event, Key},
    raw::{IntoRawMode, RawTerminal},
    screen::{AlternateScreen, IntoAlternateScreen},
    style, terminal_size,
};

type RawOut<'a> = AlternateScreen<RawTerminal<StdoutLock<'a>>>;

const BORDER: (u16, u16) = (10, 2);

const COL_SEPARATOR: &str = "        ";
const COL_SPACING: u16 = COL_SEPARATOR.len() as u16;

const HEADER_COLOR: color::Fg<color::LightGreen> = color::Fg(color::LightGreen);
const TITLE_COLOR: color::Fg<color::White> = color::Fg(color::White);
const LIST_COLOR: color::Fg<color::LightYellow> = color::Fg(color::LightYellow);
const POINTER_FG_COLOR: color::Fg<color::White> = color::Fg(color::White);
const POINTER_BG_COLOR: color::Bg<color::LightBlack> = color::Bg(color::LightBlack);
const FOOTER_COLOR: color::Fg<color::LightBlue> = color::Fg(color::LightBlue);

#[derive(Debug, Clone, Copy)]
enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy)]
struct Layout {
    header: (u16, u16),
    name: (u16, u16),
    size: (u16, u16),
    hash: (u16, u16),
    top_item: (u16, u16),
    footer: (u16, u16),
}

impl Layout {
    fn new(widths: (usize, usize, usize), n: usize, w: usize, border: (u16, u16)) -> Self {
        let half_w = terminal_size().unwrap().0 / 2;
        let cent = half_w - (w as f32 * 0.5).round() as u16;

        let header = (cent, border.1);
        let name = (cent, border.1 + 3);
        let size = (name.0 + widths.0 as u16 + COL_SPACING, border.1 + 3);
        let hash = (size.0 + widths.1 as u16 + COL_SPACING, border.1 + 3);
        let top_item = (cent - 4, border.1 + 5);
        let footer = (cent, border.1 + n as u16 + 7);

        Self {
            header,
            name,
            size,
            hash,
            top_item,
            footer,
        }
    }
}

#[derive(Clone)]
struct Interface {
    pointer: (u16, u16),
    filenames: Vec<String>,
    display: Vec<(String, bool)>,
    widths: (usize, usize, usize),
    lay: Layout,
    n: usize,
    w: usize,
    index: usize,
}

impl Interface {
    pub fn new(data: HashMap<String, (u64, String)>) -> Result<Self, Box<dyn Error>> {
        let widths = widths(&data);
        let display = display(&data, &widths);
        let n = display.len();
        let w = display[0].0.len();
        let filenames = data.keys().cloned().collect();
        let lay = Layout::new(widths, n, w, BORDER);
        let pointer = lay.top_item;

        Ok(Self {
            pointer,
            filenames,
            display,
            widths,
            lay,
            n,
            w,
            index: 0,
        })
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        // use crossbeam-channel for better performance
        let (winch_tx, winch_rx) = mpsc::channel::<()>();
        thread::spawn(move || sigwinch_handler(winch_tx).unwrap());

        let mut stdin = async_stdin().bytes();
        let mut stdout = stdout().lock().into_raw_mode()?.into_alternate_screen()?;

        self.clear(&mut stdout)?;
        self.write_layout(&mut stdout)?;
        stdout.flush()?;

        // main event loop
        loop {
            let n = stdin.next();

            if winch_rx.try_recv().is_ok() {
                self.refresh_layout();
                self.clear(&mut stdout)?;
                self.write_layout(&mut stdout)?;
                stdout.flush()?;
            }

            if let Some(Ok(k)) = n {
                let e = parse_event(k, &mut stdin);

                match e? {
                    Event::Key(Key::Char('q')) => break,
                    Event::Key(Key::Char('j')) => {
                        if self.update_pointer(Direction::Down) {
                            self.set_pointer(&mut stdout)?;
                            self.clear_pointer(&mut stdout, Direction::Down)?;
                        }
                    }
                    Event::Key(Key::Char('k')) => {
                        if self.update_pointer(Direction::Up) {
                            self.set_pointer(&mut stdout)?;
                            self.clear_pointer(&mut stdout, Direction::Up)?;
                        }
                    }
                    Event::Key(Key::Char(' ')) => {
                        self.display[self.index].1 = !self.display[self.index].1;
                        self.set_pointer(&mut stdout)?;
                    }
                    Event::Key(Key::Char('\n')) => {
                        // TODO: send the request i.e. start the dl with progress_bar
                    }
                    _ => {}
                }
            }
        }

        write!(stdout, "{}", cursor::Show).unwrap();

        Ok(())
    }

    fn clear(&self, stdout: &mut RawOut) -> Result<(), Box<dyn Error>> {
        write!(stdout, "{}{}", clear::All, cursor::Hide)?;

        Ok(())
    }

    fn write_line(
        &self,
        stdout: &mut RawOut,
        pos: &(u16, u16),
        text: String,
    ) -> Result<(), Box<dyn Error>> {
        write!(
            stdout,
            "{}{}{}",
            cursor::Goto(pos.0, pos.1),
            text,
            style::Reset
        )?;

        Ok(())
    }

    fn refresh_layout(&mut self) {
        let new_lay = Layout::new(self.widths, self.n, self.w, BORDER);
        self.lay = new_lay;
    }

    fn write_layout(&self, stdout: &mut RawOut) -> Result<(), Box<dyn Error>> {
        // header
        let header = format!(
            "{}{}Connected to the server at 123.1.2.3:8080",
            style::Bold,
            HEADER_COLOR
        );
        self.write_line(stdout, &self.lay.header, header)?;

        // footer
        let footer = format!("{}{}Press 'q' to quit", style::Bold, FOOTER_COLOR);
        self.write_line(stdout, &self.lay.footer, footer)?;

        // titles
        let name = format!("{}{}Name", style::Italic, TITLE_COLOR);
        let size = format!("{}{}Size", style::Italic, TITLE_COLOR);
        let hash = format!("{}{}SHA-256", style::Italic, TITLE_COLOR);
        self.write_line(stdout, &self.lay.name, name)?;
        self.write_line(stdout, &self.lay.size, size)?;
        self.write_line(stdout, &self.lay.hash, hash)?;

        // items
        for (i, d) in self.display.iter().enumerate() {
            let line = format!(
                "{}[{}] {}",
                LIST_COLOR,
                match d.1 {
                    true => "x",
                    false => " ",
                },
                d.0
            );
            let pos = (self.lay.top_item.0, self.lay.top_item.1 + i as u16);
            self.write_line(stdout, &pos, line)?;
        }

        // focus to the first item
        write!(stdout, "{}", cursor::Goto(self.pointer.0, self.pointer.1))?;

        Ok(())
    }

    fn clear_pointer(
        &self,
        stdout: &mut RawOut,
        direction: Direction,
    ) -> Result<(), Box<dyn Error>> {
        let (pos, text) = match direction {
            Direction::Up => (
                (self.pointer.0, self.pointer.1 + 1),
                self.display[self.index + 1].clone(),
            ),
            Direction::Down => (
                (self.pointer.0, self.pointer.1 - 1),
                self.display[self.index - 1].clone(),
            ),
        };

        let new = format!(
            "{}{}[{}] {}",
            clear::CurrentLine,
            LIST_COLOR,
            match text.1 {
                true => "x",
                false => " ",
            },
            text.0
        );
        self.write_line(stdout, &pos, new)?;
        stdout.flush()?;

        Ok(())
    }

    fn set_pointer(&self, stdout: &mut RawOut) -> Result<(), Box<dyn Error>> {
        let new = format!(
            "{}{}{}{}[{}] {}",
            clear::CurrentLine,
            style::Bold,
            POINTER_BG_COLOR,
            POINTER_FG_COLOR,
            match self.display[self.index].1 {
                true => "x",
                false => " ",
            },
            self.display[self.index].0
        );
        self.write_line(stdout, &self.pointer, new)?;
        stdout.flush()?;

        Ok(())
    }

    fn update_pointer(&mut self, direction: Direction) -> bool {
        match direction {
            Direction::Up => {
                if self.index > 0 && self.index <= self.n {
                    self.pointer.1 -= 1;
                    self.index -= 1;

                    return true;
                }
            }
            Direction::Down => {
                if self.index < self.n - 1 {
                    self.pointer.1 += 1;
                    self.index += 1;

                    return true;
                }
            }
        }

        false
    }
}

fn rand_string(limit: Option<usize>) -> String {
    let len = match limit {
        Some(limit) => limit,
        None => rand::thread_rng().gen_range(5..30),
    };
    Alphanumeric.sample_string(&mut rand::thread_rng(), len)
}

fn widths(data: &HashMap<String, (u64, String)>) -> (usize, usize, usize) {
    let mut max_name = 0;
    let mut max_size = 0;
    let mut max_hash = 0;

    data.iter().for_each(|(name, (size, hash))| {
        if name.len() > max_name {
            max_name = name.len();
        }

        if size.to_string().len() > max_size {
            max_size = size.to_string().len();
        }

        if hash.len() > max_hash {
            max_hash = hash.len();
        }
    });

    (max_name, max_size, max_hash)
}

fn display(
    data: &HashMap<String, (u64, String)>,
    widths: &(usize, usize, usize),
) -> Vec<(String, bool)> {
    let mut display = Vec::new();

    data.iter().for_each(|(name, (size, hash))| {
        let mut d = String::new();

        d.push_str(format!("{:width$}", name, width = widths.0).as_str());
        d.push_str(COL_SEPARATOR);
        d.push_str(format!("{:width$}", size, width = widths.1).as_str());
        d.push_str(COL_SEPARATOR);
        d.push_str(&format!("{}...", &hash[..20]));

        display.push((d, false));
    });

    display
}

fn sigwinch_handler(tx: Sender<()>) -> Result<(), Box<dyn Error>> {
    // for contego's async context: tokio::signal::unix::{signal, SignalKind}
    let mut signals = Signals::new([SIGWINCH])?;

    for _ in &mut signals {
        tx.send(())?;
    }

    Ok(())
}

fn main() {
    let mut data = HashMap::new();
    (0..20).into_iter().for_each(|_| {
        let filename = rand_string(None);
        let filesize = rand::thread_rng().gen_range(100..1000000);
        let hash = rand_string(Some(64));

        data.insert(filename, (filesize, hash));
    });

    let mut interface = Interface::new(data).unwrap();
    interface.run().unwrap();
}
