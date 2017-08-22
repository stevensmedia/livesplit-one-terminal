extern crate getopts;
extern crate livesplit_core;
extern crate termion;
extern crate tui;

use termion::event::Key;
use termion::input::TermRead;
use tui::Terminal;
use tui::backend::TermionBackend;
use tui::layout::{Group, Direction, Size};
use tui::widgets::{Table, Widget, Paragraph};
use tui::style::{Color, Style, Modifier};
use livesplit_core::{Timer, Run, Segment, HotkeySystem, SharedTimer};
use livesplit_core::run::parser::composite;
use livesplit_core::layout::{GeneralSettings};
use livesplit_core::component::{timer, splits, title, previous_segment, sum_of_best,
                                possible_time_save};
use std::{thread, io, env};
use std::io::BufReader;
use std::time::Duration;
use std::sync::mpsc::channel;
use std::fs::File;

struct Layout {
    timer: SharedTimer,
    components: Components,
}

struct Components {
    timer: timer::Component,
    splits: splits::Component,
    title: title::Component,
    previous_segment: previous_segment::Component,
    sum_of_best: sum_of_best::Component,
    possible_time_save: possible_time_save::Component,
}

/* Convert Livesplit display color to a tui color */
fn get_tui_color(color: livesplit_core::settings::Color) -> tui::style::Color {
    return Color::Rgb((color.rgba.red * 255.0) as u8,
                      (color.rgba.green * 255.0) as u8,
                      (color.rgba.blue * 255.0) as u8);
}

fn print_help(me: &String,  opts: &getopts::Options) {
    print!("{}", (*opts).usage(&format!("Usage: {} [options] [FILE]", me)));
    std::process::exit(0);
}

fn error_out(error: &String) {
    print!("{}\n", error);
    std::process::exit(1);
}

fn main() {
    /* Command line arguments */
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut getopts_processor = getopts::Options::new();
    getopts_processor.optflag("h", "help", "get help");

    let opt_result = getopts_processor.parse(&args[1..]);
    if opt_result.is_err() {
        print_help(&program, &getopts_processor);
    }
    let opts = opt_result.unwrap();

    if opts.opt_present("h") {
        print_help(&program, &getopts_processor);
    }

    /* Open Run if we can, otherwise default */
    let mut run = Run::new();
    if !opts.free.is_empty() {
         let ref splits_filename = &opts.free[0].clone();

         let file_result = File::open(splits_filename);
         if file_result.is_err() {
             error_out(&format!("Unable to open {}", splits_filename));
         }
         let file = file_result.unwrap();

         let run_result = composite::parse(BufReader::new(file), None, true);
         if run_result.is_err() {
             error_out(&format!("Unable to parse {}", splits_filename));
         }
         run = run_result.unwrap();
    } else {
        run.set_game_name("Livesplit Terminal");
        run.set_category_name("Any%");

        run.push_segment(Segment::new("Split 1"));
        run.push_segment(Segment::new("Split 2"));
        run.push_segment(Segment::new("Split 3"));
        run.push_segment(Segment::new("Split 4"));
        run.push_segment(Segment::new("Complete!"));
    };

    /* Create timer, layout, etc. */
    let timer = Timer::new(run).unwrap().into_shared();
    let _hotkey_system = HotkeySystem::new(timer.clone()).ok();

    let mut layout = Layout {
        timer: timer.clone(),
        components: Components {
            timer: timer::Component::new(),
            splits: splits::Component::new(),
            title: title::Component::new(),
            previous_segment: previous_segment::Component::new(),
            sum_of_best: sum_of_best::Component::new(),
            possible_time_save: possible_time_save::Component::new(),
        },
    };

    /* Set up display */
    let mut terminal = Terminal::new(TermionBackend::new().unwrap()).unwrap();

    let mut layout_settings = GeneralSettings::default();

    terminal.clear().unwrap();
    terminal.hide_cursor().unwrap();

    /* Spawn IO thread */
    let (tx, rx) = channel();

    thread::spawn(move || {
        loop {
            let stdin = io::stdin();
            for key in stdin.keys() {
                let c = key.unwrap();
                match c {
                    Key::Char('q') => break,
                    Key::Char('1') => timer.write().split_or_start(),
                    Key::Char('2') => timer.write().skip_split(),
                    Key::Char('3') => timer.write().reset(true),
                    Key::Char('4') => timer.write().switch_to_previous_comparison(),
                    Key::Char('5') => timer.write().toggle_pause_or_start(),
                    Key::Char('6') => timer.write().switch_to_next_comparison(),
                    Key::Char('8') => timer.write().undo_split(),
                    _ => {}
                }
            }
            tx.send(()).unwrap();
        }
    });

    /* Read data from IO thread while continuously updating display */
    loop {
        if let Ok(_) = rx.try_recv() {
            break;
        }

        draw(&mut terminal, &mut layout, &mut layout_settings);
        thread::sleep(Duration::from_millis(33));
    }

    terminal.clear().unwrap();
    terminal.show_cursor().unwrap();
}

/* Update display */
fn draw(t: &mut Terminal<TermionBackend>, layout: &mut Layout, layout_settings: &mut GeneralSettings) {
    let size = t.size().unwrap();

    let splits_state = layout.components.splits.state(&layout.timer.read(), layout_settings);

    Group::default()
        .margin(1)
        .sizes(&[Size::Fixed(3),
                 Size::Fixed(splits_state.splits.len() as u16 + 3),
                 Size::Fixed(2),
                 Size::Fixed(1),
                 Size::Fixed(1),
                 Size::Fixed(1)])
        .direction(Direction::Vertical)
        .render(t, &size, |t, chunks| {
            let state = layout.components.title.state(&layout.timer.read());

            let category = format!("{:^35}", match state.line2 { None => String::new(), _ => state.line2.unwrap() });
            let attempts = format!("{:>35}", state.attempts.unwrap());
            let category: String = category.chars()
                .zip(attempts.chars())
                .map(|(c, a)| if a.is_whitespace() { c } else { a })
                .collect();

            Paragraph::default()
                .text(&format!("{:^35}\n{}", state.line1, category))
                .render(t, &chunks[0]);

            let styles = splits_state.splits
                .iter()
                .map(|s| Style::default().fg(get_tui_color(s.semantic_color.visualize(layout_settings))))
                .collect::<Vec<_>>();

            let splits = splits_state.splits
                .iter()
                .zip(styles.iter())
                .map(|(s, style)| {
                    ([s.name.clone(), format!("{:>9}", s.delta), format!("{:>9}", s.time)], style)
                })
                .collect::<Vec<_>>();

            Table::default()
                .header(&["Split", "    Delta", "     Time"])
                .header_style(Style::default().fg(Color::White))
                .widths(&[15, 9, 9])
                .style(Style::default().fg(Color::White))
                .column_spacing(1)
                .rows(&splits)
                .render(t, &chunks[1]);

            let state = layout.components.timer.state(&layout.timer.read(), layout_settings);

            Paragraph::default()
                .text(&format!("{:>32}{}", state.time, state.fraction))
                .style(Style::default().modifier(Modifier::Bold).fg(get_tui_color(state.semantic_color.visualize(layout_settings))))
                .render(t, &chunks[2]);

            let state = layout.components.previous_segment.state(&layout.timer.read(), layout_settings);

            Paragraph::default()
                .text(&format_info_text(&state.text, &state.time))
                .style(Style::default().fg(get_tui_color(state.semantic_color.visualize(layout_settings))))
                .render(t, &chunks[3]);

            let state = layout.components.sum_of_best.state(&layout.timer.read());

            Paragraph::default()
                .text(&format_info_text(&state.text, &state.time))
                .style(Style::default().fg(Color::White))
                .render(t, &chunks[4]);

            let state = layout.components.possible_time_save.state(&layout.timer.read());

            Paragraph::default()
                .text(&format_info_text(&state.text, &state.time))
                .style(Style::default().fg(Color::White))
                .render(t, &chunks[5]);
        });

    t.draw().unwrap();
}

fn format_info_text(text: &str, value: &str) -> String {
    let text = format!("{:<35}", text);
    let value = format!("{:>35}", value);
    text.chars()
        .zip(value.chars())
        .map(|(t, v)| if v.is_whitespace() { t } else { v })
        .collect()
}
