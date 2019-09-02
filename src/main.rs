extern crate clap;
extern crate crossbeam;
extern crate subprocess;
extern crate termion;
extern crate tui;

#[macro_use]
extern crate lazy_static;

mod string_err;
use string_err::ToStringResult;

mod cli;
use cli::Cli;

use std::io;
use std::time;
use termion::event::{Event as UserEvent, Key, MouseButton, MouseEvent};
use termion::input::{MouseTerminal, TermRead};
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::widgets::{Block, Borders, Paragraph, Text, Widget};
use tui::Terminal;

use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use subprocess::{Popen, PopenConfig};

lazy_static! {
    static ref FLAGS: Cli = Cli::parse().unwrap();
}

static MIN_REFRESH_RATE: std::time::Duration = std::time::Duration::from_millis(1000 / 120);
static MIN_UPDATE_REFRESH_RATE: std::time::Duration = MIN_REFRESH_RATE;

struct State {
    text: Vec<String>,
    line: usize,
}

type GlobalState = Arc<Mutex<State>>;

enum Event {
    UserEvent(UserEvent),
    Update,
}

fn subprocess_chan(
    global_state: GlobalState,
    tx: crossbeam::Sender<(Event, time::Instant)>,
    rx: mpsc::Receiver<String>,
) {
    use std::sync::atomic::*;

    thread::spawn(move || -> Result<(), String> {
        let mut stop_last_thread = Arc::new(AtomicBool::new(false));
        let mut last_command = String::new();
        for command in rx.iter() {
            if last_command.trim() == command.trim() {
                continue;
            }

            stop_last_thread.store(true, Ordering::SeqCst);

            global_state
                .lock()
                .map(|mut state| {
                    state.line = 0;
                    state.text.clear();
                })
                .to_string_result()?;

            stop_last_thread = Arc::new(AtomicBool::new(false));

            // things to be moved
            let stop_thread = stop_last_thread.clone();
            let new_command = command.clone();
            let new_global_state = global_state.clone();
            let new_tx = tx.clone();

            thread::spawn(move || -> Result<(), String> {
                use std::cell::RefCell;
                use std::io::prelude::*;
                use std::rc::Rc;

                let popen_opt: Vec<Rc<Popen>> = new_command
                    .split('|')
                    .scan(
                        RefCell::new(None),
                        |acc: &mut RefCell<Option<Rc<Popen>>>, cmd| {
                            let popen = Some(Rc::new(match *acc.borrow() {
                                None => Popen::create(
                                    &["bash", "-c", cmd],
                                    PopenConfig {
                                        stderr: subprocess::Redirection::Merge,
                                        stdin: subprocess::Redirection::Pipe,
                                        stdout: subprocess::Redirection::Pipe,
                                        detached: true,
                                        ..PopenConfig::default()
                                    },
                                )
                                .ok(),
                                Some(ref prev) => Popen::create(
                                    &["bash", "-c", cmd],
                                    PopenConfig {
                                        stdin: subprocess::Redirection::File(
                                            (*prev).stdout.as_ref()?.try_clone().ok()?,
                                        ),
                                        detached: true,
                                        stdout: subprocess::Redirection::Pipe,
                                        stderr: subprocess::Redirection::Merge,
                                        ..PopenConfig::default()
                                    },
                                )
                                .ok(),
                            }?));
                            acc.replace(popen.clone());
                            popen
                        },
                    )
                    .collect();

                if popen_opt.len() == 0 {
                    return Ok(());
                }

                let output_file: std::fs::File = popen_opt[popen_opt.len() - 1]
                    .stdout
                    .as_ref()
                    .unwrap() // should never be None
                    .try_clone()
                    .to_string_result()?;

                let mut last_update = time::Instant::now();
                for line in std::io::BufReader::new(output_file).lines() {
                    let mut line = line.to_string_result()?;
                    line.push('\n');
                    if (*stop_thread).load(Ordering::SeqCst) {
                        for popen_rc in popen_opt {
                            Rc::try_unwrap(popen_rc)
                                .map_err(|_| "failed to unwrap popen".to_string())?
                                .kill()
                                .map_err(|_| "failed to kill".to_string())?;
                        }
                        break;
                    }

                    new_global_state
                        .lock()
                        .map(|mut state| {
                            state.text.push(line);
                            if !FLAGS.no_scroll {
                                state.line += 1
                            }
                        })
                        .to_string_result()?;

                    let time = time::Instant::now();
                    if time - last_update >= MIN_UPDATE_REFRESH_RATE {
                        last_update = time;
                        new_tx.send((Event::Update, time)).unwrap();
                    }
                }

                Ok(())
            });
            last_command = command;
        }
        Ok(())
    });
}

fn event_chan(tx: crossbeam::Sender<(Event, time::Instant)>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        for c in stdin.events() {
            let time = time::Instant::now();
            tx.send((Event::UserEvent(c.unwrap()), time)).unwrap();
        }
    });
}

fn text(state: &mut State, num_lines: usize) -> impl Iterator<Item = &str> {
    let len = state.text.len();
    let start_line = if state.line == len && len > 0 {
        len - num_lines.min(len)
    } else {
        state.line
    };
    state.line = start_line;
    let last_line = if len == 0 { len } else { len };
    state.text[start_line..(state.line + num_lines).min(last_line)]
        .iter()
        .map(|e| e.as_ref())
}

fn run() -> Result<String, String> {
    let stdout = io::stdout().into_raw_mode().unwrap();
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend).to_string_result()?;
    terminal.clear().to_string_result()?;
    // terminal.hide_cursor().to_string_result()?;

    let (event_tx, event_rx) = crossbeam::channel::unbounded();
    let (update_event_tx, update_event_rx) = crossbeam::channel::unbounded();
    let (command_tx, command_rx) = mpsc::channel();

    let global_state = Arc::new(Mutex::new(State {
        text: Vec::new(),
        line: 0,
    }));

    event_chan(event_tx.clone());
    subprocess_chan(global_state.clone(), update_event_tx.clone(), command_rx);

    let mut command = String::new();

    let mut last_evt = time::Instant::now();

    loop {
        match crossbeam::select! {
            recv(event_rx) -> msg => msg.map(Option::Some),
            recv(update_event_rx) -> msg => msg.map(Option::Some),
            default(std::time::Duration::from_millis(1000 / 2)) => Ok(None),
        } {
            Ok(Some((evt, _))) => match evt {
                Event::Update => (),
                Event::UserEvent(usr_evt) => match usr_evt {
                    UserEvent::Key(k) => {
                        match k {
                            Key::Ctrl('c') => break,
                            Key::Backspace => {
                                command.pop();
                            }
                            Key::Char('\r') | Key::Char('\n') => {
                                command_tx.send(command.clone()).unwrap();
                            }
                            Key::Char(ch) => command.push(ch),
                            _ => (),
                        };
                    }
                    UserEvent::Mouse(MouseEvent::Press(btn, _, _)) => match btn {
                        MouseButton::WheelDown => {
                            global_state
                                .lock()
                                .map(|mut state| {
                                    if state.line + 1 < state.text.len() {
                                        state.line += 1
                                    }
                                })
                                .to_string_result()?;
                        }
                        MouseButton::WheelUp => {
                            global_state
                                .lock()
                                .map(|mut state| {
                                    if state.line != 0 {
                                        state.line -= 1
                                    }
                                })
                                .to_string_result()?;
                        }
                        _ => (),
                    },
                    _ => (),
                },
            },
            Err(crossbeam::RecvError) => Err("dead thread")?,
            _ => (),
        }

        if time::Instant::now() - last_evt < MIN_REFRESH_RATE {
            continue;
        } else {
            last_evt = time::Instant::now();
        }

        terminal
            .draw(|mut f: tui::Frame<_>| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(0)
                    .constraints([Constraint::Length(2), Constraint::Percentage(90)].as_ref())
                    .split(f.size());

                // title bar
                let state = &mut global_state.lock().unwrap();
                let title = vec![Text::raw(&command)];
                Paragraph::new(title.iter())
                    .block(Block::default().borders(Borders::TOP | Borders::RIGHT | Borders::LEFT))
                    .wrap(true)
                    .render(&mut f, chunks[0]);

                // text drawing
                let text: Vec<Text> = text(&mut *state, chunks[1].height as usize)
                    .map(Text::raw)
                    .collect();
                Paragraph::new(text.iter())
                    .block(Block::default().borders(Borders::ALL))
                    .wrap(true)
                    .render(&mut f, chunks[1]);
            })
            .to_string_result()?;

        terminal
            .set_cursor(command.len() as u16 + 1, 1)
            .to_string_result()?;
    }
    Ok(command)
}

pub fn main() -> Result<(), String> {
    FLAGS.no_scroll;
    run().map(|command| {
        println!("{}", command);
    })
}
