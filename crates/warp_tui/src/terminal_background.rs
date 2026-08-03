//! Host terminal appearance detection and live auto-theme refresh for the TUI.
//!
//! The singleton owns the detected terminal background and the complete live
//! probe lifecycle. The foreground owns all domain state and shares only an
//! atomic eligibility gate with the runtime reader thread.

use std::io::IsTerminal;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use warp::settings::{TuiTheme, TuiThemeSettings};
use warp::tui_export::Appearance;
use warp_core::ui::theme::WarpTheme;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};
use warpui_core::runtime::{
    ProbedRgb, TuiProbe, background_luminance, probe_terminal_background,
    read_terminal_background_reply, write_terminal_background_query,
};

/// Maximum time to wait for each focus-triggered OSC 11 reply.
const LIVE_PROBE_DEADLINE: Duration = Duration::from_millis(50);
/// Consecutive missing replies allowed before probing stops for this session.
const CONSECUTIVE_MISS_CUTOFF: u8 = 3;

/// Tracks whether focus-triggered background probes may continue in this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalBackgroundProbeBudget {
    Available {
        consecutive_misses: u8,
    },
    /// The missing-reply cutoff was reached; probing cannot resume this session.
    Exhausted,
}

/// Foreground-owned terminal background and probe-budget state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalBackgroundState {
    /// Latest terminal background returned by OSC 11.
    background: Option<ProbedRgb>,
    /// Whether this session may issue further focus-triggered probes.
    probe_budget: TerminalBackgroundProbeBudget,
}
impl TerminalBackgroundState {
    fn new(background: Option<ProbedRgb>) -> Self {
        Self {
            background,
            probe_budget: TerminalBackgroundProbeBudget::Available {
                consecutive_misses: 0,
            },
        }
    }
    fn probe_enabled(&self, selected_theme: TuiTheme) -> bool {
        selected_theme == TuiTheme::Auto
            && matches!(
                self.probe_budget,
                TerminalBackgroundProbeBudget::Available { .. }
            )
    }

    fn record_probe_result(
        &mut self,
        new_background: Option<ProbedRgb>,
        current_theme: &WarpTheme,
        consecutive_miss_cutoff: u8,
    ) -> ProbeResultAction {
        let TerminalBackgroundProbeBudget::Available { consecutive_misses } =
            &mut self.probe_budget
        else {
            return ProbeResultAction::default();
        };

        let Some(new_background) = new_background else {
            *consecutive_misses = consecutive_misses.saturating_add(1);
            if *consecutive_misses >= consecutive_miss_cutoff {
                self.probe_budget = TerminalBackgroundProbeBudget::Exhausted;
            }
            return ProbeResultAction::default();
        };

        *consecutive_misses = 0;
        if self.background == Some(new_background) {
            return ProbeResultAction::default();
        }
        self.background = Some(new_background);
        let resolved_theme =
            TuiTheme::Auto.resolve_for_background(background_luminance(Some(new_background)));
        if resolved_theme != *current_theme {
            ProbeResultAction::SetTheme(resolved_theme)
        } else {
            ProbeResultAction::Repaint
        }
    }
}

/// Foreground action required after processing a background probe result.
#[derive(Clone, Debug, Default, PartialEq)]
enum ProbeResultAction {
    #[default]
    None,
    /// The exact RGB changed, so background-blended surfaces must repaint even
    /// though the resolved light/dark theme is unchanged.
    Repaint,
    /// The resolved light/dark theme changed and must be applied.
    SetTheme(WarpTheme),
}

/// Owns the host terminal appearance and live background-probe lifecycle.
#[derive(Clone, Debug)]
pub(crate) struct TuiHostTerminalBackground {
    state: TerminalBackgroundState,
    probe_enabled: Arc<AtomicBool>,
}

impl Entity for TuiHostTerminalBackground {
    type Event = ();
}

impl SingletonEntity for TuiHostTerminalBackground {}

impl TuiHostTerminalBackground {
    /// Probes the initial background, registers the singleton, and returns the
    /// resolved initial theme plus the runtime's focus-triggered two-phase probe.
    pub(crate) fn register(
        selected_theme: TuiTheme,
        ctx: &mut AppContext,
    ) -> (WarpTheme, TuiProbe) {
        let background = probe_terminal_background();
        let theme = selected_theme.resolve_for_background(background_luminance(background));
        let state = TerminalBackgroundState::new(background);
        let probe_enabled = Arc::new(AtomicBool::new(state.probe_enabled(selected_theme)));
        let reader_probe_enabled = probe_enabled.clone();
        let live_probe_supported =
            std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        let (probe_sender, probe_receiver) = async_channel::unbounded();
        ctx.add_singleton_model(move |_| Self {
            state,
            probe_enabled,
        });
        Self::handle(ctx).update(ctx, |_, ctx| {
            ctx.spawn_stream_local(
                probe_receiver,
                |appearance, background, ctx| {
                    appearance.handle_background_probe_result(background, ctx);
                },
                |_, _| {},
            );
        });

        let probe = TuiProbe::new(
            move || live_probe_supported && reader_probe_enabled.load(Ordering::Relaxed),
            probe_sender,
            |writer| write_terminal_background_query(writer),
            move || read_terminal_background_reply(LIVE_PROBE_DEADLINE),
        );
        (theme, probe)
    }

    /// Latest terminal background returned by OSC 11.
    pub(crate) fn terminal_background(&self) -> Option<ProbedRgb> {
        self.state.background
    }

    /// Updates live-probe eligibility and resolves the selected theme against
    /// the latest detected background.
    pub(crate) fn select_theme(&self, selected_theme: TuiTheme) -> WarpTheme {
        self.update_probe_enabled(selected_theme);
        selected_theme.resolve_for_background(background_luminance(self.state.background))
    }

    /// Applies one reader-thread OSC 11 result to the cached background, probe
    /// lifecycle, and active Warp theme.
    fn handle_background_probe_result(
        &mut self,
        background: Option<ProbedRgb>,
        ctx: &mut ModelContext<Self>,
    ) {
        let selected_theme = TuiThemeSettings::as_ref(ctx).selected_theme();
        if selected_theme != TuiTheme::Auto {
            self.update_probe_enabled(selected_theme);
            return;
        }
        let current_theme = Appearance::as_ref(ctx).theme().clone();
        let decision =
            self.state
                .record_probe_result(background, &current_theme, CONSECUTIVE_MISS_CUTOFF);
        self.update_probe_enabled(selected_theme);
        match decision {
            ProbeResultAction::None => {}
            ProbeResultAction::Repaint => ctx.invalidate_all_views(),
            ProbeResultAction::SetTheme(theme) => {
                Appearance::handle(ctx).update(ctx, |appearance, ctx| {
                    appearance.set_theme(theme, ctx);
                });
            }
        }
    }

    fn update_probe_enabled(&self, selected_theme: TuiTheme) {
        self.probe_enabled
            .store(self.state.probe_enabled(selected_theme), Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn register_for_test(
        background: Option<ProbedRgb>,
        selected_theme: TuiTheme,
        ctx: &mut AppContext,
    ) {
        let state = TerminalBackgroundState::new(background);
        let probe_enabled = Arc::new(AtomicBool::new(state.probe_enabled(selected_theme)));
        ctx.add_singleton_model(move |_| Self {
            state,
            probe_enabled,
        });
    }
}

#[cfg(test)]
#[path = "terminal_background_tests.rs"]
mod tests;
