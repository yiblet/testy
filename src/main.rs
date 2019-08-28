extern crate either;
extern crate subprocess;
extern crate termion;
extern crate tui;

use std::io;
use termion::event::{Event as UserEvent, Key, MouseEvent};
use termion::input::{MouseTerminal, TermRead};
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::widgets::{Block, Borders, Paragraph, Text, Widget};
use tui::Terminal;

use either::Either;
use std::sync::mpsc;
use std::thread;
use subprocess::{Popen, PopenConfig};

enum Event {
    UserEvent(UserEvent),
    OutputLine(String),
}

fn subprocess_chan(tx: mpsc::Sender<Event>, rx: mpsc::Receiver<String>) {
    use std::sync::atomic::*;
    use std::sync::Arc;

    thread::spawn(move || -> Result<(), String> {
        let mut stop_last_thread = Arc::new(AtomicBool::new(false));
        let mut last_command = String::new();
        for command in rx.iter() {
            let thread_tx = tx.clone();
            if last_command.trim() == command.trim() {
                continue;
            }

            stop_last_thread.store(true, Ordering::SeqCst);

            stop_last_thread = Arc::new(AtomicBool::new(false));

            // things to be moved
            let stop_thread = stop_last_thread.clone();
            let new_command = command.clone();

            thread::spawn(move || -> Result<(), String> {
                use std::io::prelude::*;
                let popen_opt = new_command
                    .split('|')
                    .fold(None, |acc: Option<Popen>, cmd| {
                        (match acc {
                            None => Popen::create(
                                &["bash", "-c", cmd],
                                PopenConfig {
                                    stderr: subprocess::Redirection::Pipe,
                                    stdin: subprocess::Redirection::None,
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
                                        prev.stdout.as_ref()?.try_clone().ok()?,
                                    ),
                                    detached: true,
                                    stdout: subprocess::Redirection::Pipe,
                                    stderr: subprocess::Redirection::Pipe,
                                    ..PopenConfig::default()
                                },
                            )
                            .ok(),
                        })
                    });

                let output_file: std::fs::File = popen_opt
                    .ok_or("popen failed".to_string())
                    .and_then(|pop| {
                        pop.stdout
                            .as_ref()
                            .unwrap() // should never be None
                            .try_clone()
                            .map_err(|e| e.to_string())
                    })?;

                for line in std::io::BufReader::new(output_file).lines() {
                    let line = line.map_err(|e| e.to_string())?;
                    if (*stop_thread).load(Ordering::SeqCst) {
                        break;
                    }
                    thread_tx
                        .send(Event::OutputLine(line))
                        .map_err(|e| e.to_string())?;
                }

                Ok(())
            });
            last_command = command;
        }
        Ok(())
    });
}

fn event_chan(tx: mpsc::Sender<Event>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        for c in stdin.events() {
            tx.send(Event::UserEvent(c.unwrap())).unwrap();
        }
    });
}

fn main() -> Result<(), String> {
    let stdout = io::stdout().into_raw_mode().unwrap();
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;
    terminal.clear().map_err(|e| e.to_string())?;
    terminal.hide_cursor();

    let (event_tx, event_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();

    event_chan(event_tx.clone());
    subprocess_chan(event_tx.clone(), command_rx);

    let mut command = String::new();
    let mut output = String::new();
    loop {
        match event_rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(evt) => match evt {
                Event::UserEvent(usr_evt) => match usr_evt {
                    UserEvent::Key(k) => {
                        match k {
                            Key::Ctrl('c') => break,
                            Key::Backspace => {
                                command.pop();
                            }
                            Key::Char('\r') | Key::Char('\n') => {
                                command_tx.send(command.clone()).unwrap();
                                output.clear();
                            }
                            Key::Char(ch) => command.push(ch),
                            _ => (),
                        };
                    }
                    _ => (),
                },
                Event::OutputLine(line) => {
                    output.push_str(line.as_ref());
                    output.push('\n');
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("dead thread")?,
            _ => (),
        }

        terminal
            .draw(|mut f| {
                let text = [Text::raw(output.clone())];
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(0)
                    .constraints([Constraint::Length(1), Constraint::Percentage(90)].as_ref())
                    .split(f.size());
                let block = Block::default()
                    .title(command.as_ref())
                    .borders(Borders::ALL);
                Paragraph::new(text.iter())
                    .block(block)
                    .wrap(true)
                    .render(&mut f, chunks[1]);
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
