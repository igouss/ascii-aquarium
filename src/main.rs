//! ascii-sharks and ascii-koi as a screensaver, in a fullscreen GPUI window.
//! With `--app-id` it acts as the Omarchy screensaver.

mod adapters;
mod app;
mod domain;
mod ports;

use std::{
    io::{self, Write as _},
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use clap::{Parser, ValueEnum};

use crate::{
    adapters::{
        hyprland::{HiddenPointer, Hyprland},
        journal::Journal,
        siblings,
        window::{self, Exits},
    },
    app::Session,
    domain::{
        input::StopReason,
        koi::{self, KoiKind},
        koi_skin::Theme,
        scene::Kind,
        shark::{self, SharkKind},
    },
};

/// How often the screensaver asks Hyprland whether it still has focus.
const PRESENCE_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// The scene: `sharks` or `koi`, or a path whose file name (without
    /// extension) is one, such as the rotation's `sharks.ascii`
    scene: PathBuf,

    /// Who swims, biggest first, as upstream's `?sharks=` and `?koi=` take
    /// them: sharks from classic, hammerhead, zebra, tiger, greathammer, mako,
    /// thresher, nurse; koi from kohaku, orange, benigoi, tancho, butterfly
    /// [default: every shark kind, in that order; upstream's four koi]
    #[arg(long, value_delimiter = ',', value_name = "KINDS")]
    cast: Vec<String>,

    /// The koi pond's water
    #[arg(long, value_enum, default_value_t = ThemeArg::Dark)]
    theme: ThemeArg,

    /// Frames per second (0 = 60); the simulation always steps at 60
    #[arg(long, default_value_t = 30)]
    max_fps: u32,

    /// Seed for the scene
    #[arg(long)]
    seed: Option<u64>,

    /// Run as the screensaver, in a terminal window with this app id: hide the
    /// pointer, stop once no such window has focus or the session locks, and
    /// stop the other monitors' instances along with this one
    #[arg(long, value_name = "ID")]
    app_id: Option<String>,

    /// Append "showing" and "stopped" entries to the shader screensaver's run log
    #[arg(long, value_name = "FILE")]
    log: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ThemeArg {
    Dark,
    Light,
}

fn main() -> ExitCode {
    match run(&Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // Best effort: stderr may be the terminal that just went away.
            let _ = writeln!(io::stderr(), "ascii-aquarium: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    let theme = match cli.theme {
        ThemeArg::Dark => Theme::Dark,
        ThemeArg::Light => Theme::Light,
    };
    let unknown = |name: &String| format!("no {name} in this scene");
    let kind = match cli.scene.file_stem().and_then(|s| s.to_str()) {
        Some("sharks") if cli.cast.is_empty() => Kind::Sharks(shark::cast(&SharkKind::ALL)),
        Some("sharks") => Kind::Sharks(shark::cast(
            &cli.cast
                .iter()
                .map(|n| SharkKind::named(n).ok_or_else(|| unknown(n)))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Some("koi") if cli.cast.is_empty() => Kind::Koi(koi::DEFAULT_KOI.to_vec(), theme),
        Some("koi") => Kind::Koi(
            koi::cast(
                &cli.cast
                    .iter()
                    .map(|n| KoiKind::named(n).ok_or_else(|| unknown(n)))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            theme,
        ),
        _ => {
            return Err(format!(
                "no scene named {}: try sharks or koi",
                cli.scene.display()
            ));
        }
    };
    let fps = if cli.max_fps == 0 {
        60
    } else {
        cli.max_fps.min(60)
    };
    let seed = cli.seed.unwrap_or_else(|| fastrand::u64(..));

    let stop = Arc::new(AtomicBool::new(false));
    let on_signal = Arc::clone(&stop);
    ctrlc::set_handler(move || on_signal.store(true, Ordering::Relaxed))
        .map_err(|error| format!("can't handle signals: {error}"))?;

    let journal = cli.log.as_deref().map(Journal::new);
    if let Some(journal) = &journal {
        journal
            .showing(&cli.scene)
            .map_err(|e| format!("--log: {e}"))?;
    }

    let screensaver = cli.app_id.is_some();
    let focus_left = Arc::new(AtomicBool::new(false));
    if let Some(id) = &cli.app_id {
        let presence = Hyprland::new(id.clone());
        let left = Arc::clone(&focus_left);
        thread::spawn(move || {
            let started = Instant::now();
            app::watch(
                presence,
                &left,
                || started.elapsed(),
                || thread::sleep(PRESENCE_POLL),
            )
        });
    }
    let pointer = screensaver.then(HiddenPointer::new);
    let started = Instant::now();
    let on_stop = move |reason: StopReason| {
        drop(pointer);
        if let Some(journal) = &journal
            && let Err(e) = journal.stopped(started.elapsed(), reason)
        {
            let _ = writeln!(io::stderr(), "ascii-aquarium: --log: {e}");
        }
        // Dismissing one monitor's screensaver dismisses them all. Those
        // stopped by a signal (likely from a sibling) leave the rest be.
        if screensaver && reason != StopReason::WindowClosed {
            siblings::terminate_siblings();
        }
    };
    window::run(
        Session::new(kind, seed),
        fps,
        cli.app_id
            .clone()
            .unwrap_or_else(|| "ascii-aquarium".to_owned()),
        Exits {
            stop,
            focus_left,
            on_stop: Box::new(on_stop),
        },
    );
    Ok(())
}
