use super::{ClosePlan, ControlClientLoss, PaneRegistry, TmuxViewSlots};
use crate::features::FeatureFlag;
use crate::pane_group::NewTerminalOptions;
use crate::terminal::TerminalModel;
use crate::terminal::tmux::parser::PaneId;

#[test]
fn output_is_delivered_to_registered_panes_only() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    registry.register(PaneId::from("%1"));
    assert!(registry.should_deliver_output(&PaneId::from("%0")));
    assert!(registry.should_deliver_output(&PaneId::from("%1")));
    assert!(!registry.should_deliver_output(&PaneId::from("%2")));
}

#[test]
fn first_registered_pane_is_focused_until_select() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    registry.register(PaneId::from("%1"));
    assert_eq!(registry.focused().map(PaneId::as_str), Some("%0"));
    assert!(registry.focus(&PaneId::from("%1")));
    assert_eq!(registry.focused().map(PaneId::as_str), Some("%1"));
    assert!(!registry.focus(&PaneId::from("%9")));
}

#[test]
fn closing_a_sibling_keeps_the_session() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    registry.register(PaneId::from("%1"));
    assert_eq!(
        registry.close_plan(&PaneId::from("%1")),
        ClosePlan::KillPane
    );
    assert_eq!(
        registry.unregister(&PaneId::from("%1")),
        ClosePlan::KillPane
    );
    assert!(registry.contains(&PaneId::from("%0")));
    assert_eq!(
        registry.close_plan(&PaneId::from("%0")),
        ClosePlan::DetachClient
    );
}

#[test]
fn closing_the_last_pane_detaches_the_client() {
    let mut registry = PaneRegistry::new();
    registry.register(PaneId::from("%0"));
    assert_eq!(
        registry.unregister(&PaneId::from("%0")),
        ClosePlan::DetachClient
    );
    assert!(registry.is_empty());
}

#[test]
fn two_pane_ids_materialize_two_view_slots() {
    let mut views = TmuxViewSlots::default();
    views.deliver(PaneId::from("%0"), b"one");
    views.deliver(PaneId::from("%1"), b"two");
    views.deliver(PaneId::from("%0"), b"+");
    assert_eq!(views.view_count(), 2);
    assert_eq!(views.output(&PaneId::from("%0")), Some(b"one+".as_slice()));
    assert_eq!(views.output(&PaneId::from("%1")), Some(b"two".as_slice()));
}

#[test]
fn transport_eof_never_kills_the_session() {
    assert_eq!(
        ControlClientLoss::TransportEof.close_plan(true),
        ClosePlan::DetachClient
    );
    assert_eq!(
        ControlClientLoss::ExplicitClose.close_plan(true),
        ClosePlan::DetachClient
    );
    assert_eq!(
        ControlClientLoss::ExplicitClose.close_plan(false),
        ClosePlan::KillPane
    );
}

#[test]
fn unknown_pane_close_is_a_no_op() {
    let registry = PaneRegistry::new();
    assert_eq!(
        registry.close_plan(&PaneId::from("%0")),
        ClosePlan::UnknownPane
    );
}

#[test]
fn gateway_requests_a_presentation_window_once() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_control_mode(true);
    assert!(model.take_tmux_open_presentation());
    assert!(!model.take_tmux_open_presentation());
}

#[test]
fn presentation_models_do_not_open_another_window() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.set_tmux_control_mode(true);
    assert!(model.is_tmux_presentation());
    assert!(!model.take_tmux_open_presentation());
}

#[test]
fn gateway_exit_requests_presentation_window_close() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_control_mode(true);
    let _ = model.take_tmux_open_presentation();
    model.set_tmux_control_mode(false);
    assert!(model.take_tmux_close_presentation());
    assert!(!model.take_tmux_close_presentation());
}

#[test]
fn control_mode_exit_keeps_instance_id_for_close() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_instance_id(Some(42));
    model.set_tmux_control_mode(true);
    let _ = model.take_tmux_open_presentation();
    model.set_tmux_control_mode(false);
    assert_eq!(model.tmux_instance_id(), Some(42));
    assert!(model.take_tmux_close_presentation());
}

#[cfg(all(unix, feature = "local_tty", not(feature = "remote_tty")))]
#[test]
fn enter_exit_close_enter_creates_and_binds_a_fresh_runtime() {
    use warp_terminal::local_tty::event_loop::ActiveTerminal;

    use crate::terminal::tmux::bridge::{TmuxInstanceId, TmuxRuntime};

    let mut model = TerminalModel::mock(None, None);
    model.on_tmux_control_mode(true);
    let first = model.tmux_instance_id().expect("runtime bound on enter");
    assert!(TmuxRuntime::for_id(TmuxInstanceId::from_u64(first)).is_some());
    assert!(model.take_tmux_open_presentation());

    model.on_tmux_control_mode(false);
    assert_eq!(model.tmux_instance_id(), Some(first));
    assert!(model.take_tmux_close_presentation());

    TmuxRuntime::for_id(TmuxInstanceId::from_u64(first))
        .expect("runtime still registered for close")
        .unregister();
    model.set_tmux_instance_id(None);

    model.on_tmux_control_mode(true);
    let second = model.tmux_instance_id().expect("fresh runtime on re-enter");
    assert_ne!(first, second);
    assert!(TmuxRuntime::for_id(TmuxInstanceId::from_u64(second)).is_some());
    TmuxRuntime::for_id(TmuxInstanceId::from_u64(second))
        .unwrap()
        .unregister();
}

#[cfg(all(unix, feature = "local_tty", not(feature = "remote_tty")))]
#[test]
fn reenter_with_stale_instance_id_creates_a_fresh_runtime() {
    use warp_terminal::local_tty::event_loop::ActiveTerminal;

    use crate::terminal::tmux::bridge::{TmuxInstanceId, TmuxRuntime};

    let mut model = TerminalModel::mock(None, None);
    model.on_tmux_control_mode(true);
    let first = model.tmux_instance_id().expect("runtime bound on enter");
    let _ = model.take_tmux_open_presentation();

    model.on_tmux_control_mode(false);
    assert_eq!(model.tmux_instance_id(), Some(first));
    TmuxRuntime::for_id(TmuxInstanceId::from_u64(first))
        .unwrap()
        .unregister();

    model.on_tmux_control_mode(true);
    let second = model.tmux_instance_id().expect("fresh runtime on re-enter");
    assert_ne!(first, second);
    assert!(TmuxRuntime::for_id(TmuxInstanceId::from_u64(second)).is_some());
    TmuxRuntime::for_id(TmuxInstanceId::from_u64(second))
        .unwrap()
        .unregister();
}

#[test]
fn default_new_terminal_options_are_not_tmux_owned() {
    assert!(!NewTerminalOptions::default().tmux_presentation);
}

#[test]
fn presentation_pane_id_is_explicit() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    assert_eq!(model.tmux_pane_id(), None);
    model.set_tmux_pane_id(Some("%7".to_owned()));
    assert_eq!(model.tmux_pane_id(), Some("%7"));
}

#[test]
fn presentation_output_paints_on_the_alt_screen_grid() {
    use crate::terminal::model::ansi::{Handler, Mode};
    use crate::terminal::model::terminal_model::TerminalInputState;

    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    assert!(
        model.is_alt_screen_active(),
        "presentation must take the alt-screen paint path"
    );
    assert!(matches!(
        model.terminal_input_state(),
        TerminalInputState::AltScreen
    ));
    model.process_bytes("prompt $ hello-from-tmux");
    let painted = model.alt_screen().output_to_string();
    assert!(
        painted.contains("hello-from-tmux"),
        "populated grid must be on the alt-screen paint path, got {painted:?}"
    );
    model.unset_mode(Mode::SwapScreen {
        save_cursor_and_clear_screen: true,
    });
    model.finish_block();
    assert!(
        model.is_alt_screen_active(),
        "presentation must keep the alt-screen paint path after Warp hook alt-screen exit"
    );
    assert!(
        model
            .alt_screen()
            .output_to_string()
            .contains("hello-from-tmux"),
        "primary grid must survive Warp hook alt-screen exit, got {:?}",
        model.alt_screen().output_to_string()
    );
}

#[test]
fn presentation_nested_alt_screen_restores_primary_grid() {
    use crate::terminal::model::ansi::{Handler, Mode};

    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.process_bytes("prompt $ hello-primary");
    assert!(
        model
            .alt_screen()
            .output_to_string()
            .contains("hello-primary"),
        "primary prompt must be painted, got {:?}",
        model.alt_screen().output_to_string()
    );

    model.set_mode(Mode::SwapScreen {
        save_cursor_and_clear_screen: true,
    });
    assert!(model.is_alt_screen_active());
    assert!(
        !model
            .alt_screen()
            .output_to_string()
            .contains("hello-primary"),
        "CSI ?1049h must leave the primary grid, got {:?}",
        model.alt_screen().output_to_string()
    );
    model.process_bytes("vim-tui");
    let tui = model.alt_screen().output_to_string();
    assert!(
        tui.contains("vim-tui"),
        "nested TUI must draw on the alternate buffer, got {tui:?}"
    );
    assert!(!tui.contains("hello-primary"));

    model.unset_mode(Mode::SwapScreen {
        save_cursor_and_clear_screen: true,
    });
    assert!(model.is_alt_screen_active());
    let restored = model.alt_screen().output_to_string();
    assert!(
        restored.contains("hello-primary"),
        "CSI ?1049l must restore the primary grid, got {restored:?}"
    );
    assert!(
        !restored.contains("vim-tui"),
        "restored primary must not keep TUI contents, got {restored:?}"
    );

    model.process_bytes(" more");
    let live = model.alt_screen().output_to_string();
    assert!(
        live.contains("hello-primary") && live.contains("more"),
        "live updates must continue on the restored primary grid, got {live:?}"
    );
}

#[test]
fn presentation_nested_csi_47_uses_a_distinct_alt_grid() {
    use crate::terminal::model::ansi::{Handler, Mode};

    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.process_bytes("prompt $ hello-primary");
    assert!(
        model
            .alt_screen()
            .output_to_string()
            .contains("hello-primary"),
        "primary prompt must be painted, got {:?}",
        model.alt_screen().output_to_string()
    );

    model.set_mode(Mode::SwapScreen {
        save_cursor_and_clear_screen: false,
    });
    assert!(model.is_alt_screen_active());
    assert!(
        !model
            .alt_screen()
            .output_to_string()
            .contains("hello-primary"),
        "CSI ?47h must leave the primary grid immediately, got {:?}",
        model.alt_screen().output_to_string()
    );
    model.process_bytes("alt-program");
    let tui = model.alt_screen().output_to_string();
    assert!(
        tui.contains("alt-program"),
        "nested program must draw on the alternate buffer, got {tui:?}"
    );
    assert!(!tui.contains("hello-primary"));

    model.unset_mode(Mode::SwapScreen {
        save_cursor_and_clear_screen: false,
    });
    assert!(model.is_alt_screen_active());
    let restored = model.alt_screen().output_to_string();
    assert!(
        restored.contains("hello-primary"),
        "CSI ?47l must restore the owned primary grid, got {restored:?}"
    );
    assert!(
        !restored.contains("alt-program"),
        "restored primary must not keep nested contents, got {restored:?}"
    );
}

#[test]
fn presentation_nested_alt_preserves_cell_metrics() {
    use pathfinder_geometry::vector::vec2f;
    use warpui::units::{IntoPixels as _, Pixels};

    use crate::terminal::model::ansi::{Handler, Mode};
    use crate::terminal::{SizeInfo, SizeUpdate, SizeUpdateReason};

    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    let last_size = SizeInfo::new_without_font_metrics(10, 7);
    let new_size = SizeInfo::new(
        vec2f(800.0, 480.0),
        10.0.into_pixels(),
        20.0.into_pixels(),
        Pixels::zero(),
        Pixels::zero(),
    );
    model.resize(SizeUpdate {
        update_reason: SizeUpdateReason::Refresh,
        last_size,
        new_size,
        new_gap_height: None,
        natural_rows: new_size.rows(),
        natural_cols: new_size.columns(),
    });
    model.set_mode(Mode::SwapScreen {
        save_cursor_and_clear_screen: false,
    });
    let mut response = Vec::new();
    model.text_area_size_pixels(&mut response);
    assert_eq!(
        response.as_slice(),
        b"\x1b[4;480;800t",
        "nested alt-screen must keep cell pixel metrics, got {:?}",
        String::from_utf8_lossy(&response)
    );
}

#[test]
fn presentation_nested_alt_hides_primary_images() {
    use crate::terminal::model::ansi::{Handler, Mode};
    use crate::terminal::model::test_utils::test_iterm_image;

    let _iterm = FeatureFlag::ITermImages.override_enabled(true);
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.handle_completed_iterm_image(test_iterm_image(7));
    assert!(
        model.alt_screen().grid_handler().has_visible_images(),
        "primary grid must keep the iTerm image"
    );
    model.set_mode(Mode::SwapScreen {
        save_cursor_and_clear_screen: false,
    });
    assert!(
        !model.alt_screen().grid_handler().has_visible_images(),
        "nested alt-screen must not show the primary image"
    );
    model.unset_mode(Mode::SwapScreen {
        save_cursor_and_clear_screen: false,
    });
    assert!(
        model.alt_screen().grid_handler().has_visible_images(),
        "exiting nested alt-screen must restore the primary image"
    );
}

#[test]
fn presentation_grid_clear_drops_bootstrap_markers() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.process_bytes("warp_bootstrapped() {\nEOM\nprintf DCS\n");
    assert!(
        model
            .alt_screen()
            .output_to_string()
            .contains("warp_bootstrapped"),
        "setup text is present before clear, got {:?}",
        model.alt_screen().output_to_string()
    );
    model.process_bytes("\x1b[H\x1b[2Jprompt $ ");
    let painted = model.alt_screen().output_to_string();
    assert!(
        !painted.contains("warp_bootstrapped") && !painted.contains("EOM"),
        "cleared grid must not keep bootstrap markers, got {painted:?}"
    );
    assert!(
        painted.contains("prompt $"),
        "cleared grid should show the prompt, got {painted:?}"
    );
}

#[cfg(all(unix, feature = "local_tty", not(feature = "remote_tty")))]
#[test]
fn bound_presentation_pane_output_paints_on_the_alt_screen_grid() {
    use std::sync::Arc;

    use parking_lot::FairMutex;

    use crate::terminal::model::terminal_model::TerminalInputState;
    use crate::terminal::tmux::bridge::TmuxRuntime;

    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.set_tmux_pane_id(Some("%0".to_owned()));
    let model = Arc::new(FairMutex::new(model));
    let runtime = TmuxRuntime::new();
    runtime.register_pane("%0", model.clone());
    assert!(runtime.deliver_output(&PaneId::from("%0"), b"zsh% live-grid"));
    {
        let locked = model.lock();
        assert!(locked.is_alt_screen_active());
        assert!(matches!(
            locked.terminal_input_state(),
            TerminalInputState::AltScreen
        ));
        let painted = locked.alt_screen().output_to_string();
        assert!(
            painted.contains("live-grid"),
            "bound %0 snapshot/%output must paint on the alt-screen grid, got {painted:?}"
        );
    }
    runtime.unregister();
}

#[test]
fn init_shell_remote_bash_is_authoritative_over_local_zsh_launch() {
    use crate::terminal::ShellLaunchState;
    use crate::terminal::model::ansi::{Handler, InitShellValue};
    use crate::terminal::model::session::get_local_hostname;
    use crate::terminal::shell::ShellType;

    let mut model = TerminalModel::mock(None, None);
    assert_eq!(model.last_init_shell_type(), Some(ShellType::Zsh));
    match model.shell_launch_state() {
        ShellLaunchState::ShellSpawned { shell_type, .. } => {
            assert_eq!(*shell_type, ShellType::Zsh);
        }
        other => panic!("unexpected {other:?}"),
    }
    let session_id = 77.into();
    model.register_session_id(session_id);
    let hostname = get_local_hostname().unwrap_or_else(|_| "localhost".to_string());
    model.init_shell(InitShellValue {
        session_id,
        shell: "bash".to_owned(),
        hostname,
        ..Default::default()
    });
    assert_eq!(model.last_init_shell_type(), Some(ShellType::Bash));
    match model.shell_launch_state() {
        ShellLaunchState::ShellSpawned { shell_type, .. } => {
            assert_eq!(*shell_type, ShellType::Zsh);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn presentation_init_shell_hook_does_not_bypass_readiness() {
    use crate::terminal::model::ansi::{Handler, InitShellValue};
    use crate::terminal::model::session::get_local_hostname;

    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.set_tmux_pane_id(Some("%0".to_owned()));
    let session_id = 4242.into();
    model.register_session_id(session_id);
    let hostname = get_local_hostname().unwrap_or_else(|_| "localhost".to_string());
    model.init_shell(InitShellValue {
        session_id,
        shell: "zsh".to_owned(),
        hostname,
        ..Default::default()
    });
    assert_eq!(model.pending_session_id(), Some(session_id));
}

#[test]
fn layout_events_are_queued_until_taken() {
    use crate::terminal::model::terminal_model::TmuxClientEvent;
    let mut model = TerminalModel::mock(None, None);
    model.push_tmux_event(TmuxClientEvent::WindowAdd {
        window_id: "@3".to_owned(),
    });
    let events = model.take_tmux_events();
    assert_eq!(
        events,
        vec![TmuxClientEvent::WindowAdd {
            window_id: "@3".to_owned()
        }]
    );
    assert!(model.take_tmux_events().is_empty());
}

#[test]
fn presentation_split_uses_bound_pane_id_not_unset_focus() {
    use crate::terminal::tmux::parser::PaneId;
    use crate::terminal::tmux::protocol::split_window_command;
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_presentation(true);
    model.set_tmux_control_mode(true);
    assert!(model.tmux_split_target_pane().is_none());
    model.set_tmux_pane_id(Some("%5".to_owned()));
    assert_eq!(model.tmux_split_target_pane(), Some("%5"));
    let target = PaneId::from(model.tmux_split_target_pane().unwrap());
    assert_eq!(
        split_window_command(&target, true),
        "split-window -h -t %5 -P -F '#{pane_id}'\n"
    );
    assert_eq!(
        split_window_command(&target, false),
        "split-window -v -t %5 -P -F '#{pane_id}'\n"
    );
}

#[test]
fn gateway_split_falls_back_to_focused_pane() {
    let mut model = TerminalModel::mock(None, None);
    model.set_tmux_control_mode(true);
    model.set_tmux_focused_pane(Some("%3".to_owned()));
    assert_eq!(model.tmux_split_target_pane(), Some("%3"));
}

#[test]
fn expected_session_id_is_stored_on_gateway_model() {
    use crate::terminal::model::session::SessionId;
    let mut model = TerminalModel::mock(None, None);
    let session_id = SessionId::from(99);
    model.set_tmux_expected_session_id(Some(session_id));
    assert_eq!(model.tmux_expected_session_id(), Some(session_id));
}

#[cfg(all(unix, feature = "local_tty", not(feature = "remote_tty")))]
#[test]
fn in_place_bind_tracks_control_pane_and_expected_session() {
    use crate::terminal::model::session::SessionId;
    use crate::terminal::tmux::bridge::TmuxRuntime;
    let session_id = SessionId::from(77);
    let runtime = TmuxRuntime::new();
    runtime.note_tracked_control_pane("%0");
    runtime.set_tracked_expected_session(session_id);
    assert_eq!(runtime.tracked_control_pane().as_deref(), Some("%0"));
    assert_eq!(runtime.tracked_expected_session(), Some(session_id));
    runtime.unregister();
}

#[test]
fn two_models_keep_independent_instance_ids() {
    let mut a = TerminalModel::mock(None, None);
    let mut b = TerminalModel::mock(None, None);
    a.set_tmux_instance_id(Some(1));
    b.set_tmux_instance_id(Some(2));
    a.set_tmux_pane_id(Some("%0".to_owned()));
    b.set_tmux_pane_id(Some("%0".to_owned()));
    assert_eq!(a.tmux_instance_id(), Some(1));
    assert_eq!(b.tmux_instance_id(), Some(2));
    a.set_tmux_instance_id(None);
    assert_eq!(b.tmux_instance_id(), Some(2));
    assert_eq!(b.tmux_pane_id(), Some("%0"));
}

#[test]
fn feature_off_does_not_treat_panes_as_tmux_owned() {
    let _flag = FeatureFlag::TmuxControlPrototype.override_enabled(false);
    let model = TerminalModel::mock(None, None);
    assert!(!model.is_tmux_control_mode());
    assert!(!model.is_tmux_presentation());
    assert!(!NewTerminalOptions::default().tmux_presentation);
}

#[cfg(all(unix, feature = "local_tty", not(feature = "remote_tty")))]
#[test]
fn tmux_36a_control_bytes_open_and_queue_layout_on_gateway_model() {
    use warp_terminal::local_tty::event_loop::ActiveTerminal;
    use warp_terminal::tmux::{CONTROL_MODE_DCS, TmuxFeedItem, TmuxIoState};

    use crate::terminal::model::terminal_model::TmuxClientEvent;
    use crate::terminal::tmux::bridge::{TmuxInstanceId, TmuxRuntime};

    let mut io = TmuxIoState::new();
    io.enqueue_input(std::borrow::Cow::Borrowed(
        b"tmux -CC new-session -A -s warp -n warp -x 80 -y 24\n",
    ));
    let mut bytes = CONTROL_MODE_DCS.to_vec();
    bytes.extend_from_slice(
        b"%begin 271 0\n%end 271 0\n%window-add @0\n%sessions-changed\n%session-changed $0 warp\n%output %0 hi\n",
    );
    let items = io.feed(&bytes);

    let mut model = TerminalModel::mock(None, None);
    for item in items {
        match item {
            TmuxFeedItem::EnteredControl { .. } => model.on_tmux_control_mode(true),
            TmuxFeedItem::WindowAdd { window_id } => model.on_tmux_window_add(&window_id),
            TmuxFeedItem::LayoutChange {
                window_id,
                layout,
                visible_layout,
                flags,
            } => model.on_tmux_layout(
                &window_id,
                &layout,
                visible_layout.as_deref(),
                flags.as_deref(),
            ),
            TmuxFeedItem::PaneOutput { pane_id, bytes } => {
                let _ = model.on_tmux_pane_output(&pane_id, &bytes);
            }
            _ => {}
        }
    }
    assert!(model.take_tmux_open_presentation());
    let events = model.take_tmux_events();
    assert!(events.iter().any(|event| matches!(
        event,
        TmuxClientEvent::WindowAdd { window_id } if window_id == "@0"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        TmuxClientEvent::LayoutChange { window_id, layout, .. }
            if window_id == "@0" && layout == "80x24,0,0,0"
    )));
    if let Some(id) = model.tmux_instance_id() {
        TmuxRuntime::for_id(TmuxInstanceId::from_u64(id))
            .unwrap()
            .unregister();
    }
}

#[cfg(all(unix, feature = "local_tty", not(feature = "remote_tty")))]
#[test]
fn bare_tmux_dispatches_retained_zsh_init_and_skips_fallback() {
    use warp_terminal::local_tty::event_loop::ActiveTerminal;
    use warp_terminal::tmux::{
        CONTROL_MODE_DCS, EXIT_EMPTY_OFF_COMMAND, TmuxFeedItem, TmuxIoState,
    };

    use crate::terminal::shell::ShellType;
    use crate::terminal::tmux::bridge::{TmuxInstanceId, TmuxRuntime};
    use crate::terminal::tmux::parser::PaneId;
    use crate::terminal::tmux::protocol::{send_keys_commands, zsh_init_bytes};
    use crate::terminal::tmux::transport::tmux_cc_shell_command;

    let (command, spawned, zsh_init) =
        tmux_cc_shell_command("", Some("warp"), 80, 24, Some(ShellType::Zsh)).expect("bare /tmux");
    let spawned = spawned.expect("bare /tmux always includes a pane spawn");
    let zsh_init = zsh_init.expect("zsh no-rcs pane keeps retained init");
    assert!(command.contains("-L warp-control-v1"));
    assert!(command.contains("--no-rcs"));
    let expected_keys = zsh_init_bytes(&zsh_init, ShellType::Zsh, spawned);
    assert!(String::from_utf8_lossy(&expected_keys).contains("InitShell"));

    let mut gateway = TerminalModel::mock(None, None);
    gateway.set_tmux_expected_session_id(Some(spawned));
    gateway.register_session_id(spawned);
    gateway.set_tmux_retained_zsh_init(Some(zsh_init));

    let mut io = TmuxIoState::new();
    io.enqueue_input(std::borrow::Cow::Owned(command.into_bytes()));
    let items = io.feed(CONTROL_MODE_DCS);
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(bytes)
            if bytes.as_slice() == EXIT_EMPTY_OFF_COMMAND.as_bytes()
    )));

    gateway.on_tmux_control_mode(true);
    assert!(gateway.tmux_expected_session_id().is_none());
    let id = gateway.tmux_instance_id().expect("runtime bound on enter");
    let runtime = TmuxRuntime::for_id(TmuxInstanceId::from_u64(id)).expect("runtime");
    assert_eq!(runtime.spawned_expected_session(), Some(spawned));

    let writes = runtime.take_retained_init_send_keys("%0");
    let expected_commands = send_keys_commands(&PaneId::from("%0"), &expected_keys);
    assert_eq!(
        writes,
        expected_commands
            .into_iter()
            .map(String::into_bytes)
            .collect::<Vec<_>>()
    );
    assert!(runtime.control_pane_owns_retained_init("%0"));
    assert_eq!(runtime.take_retained_init_send_keys("%0").len(), 0);

    runtime
        .begin_pane_bootstrap("%0", spawned)
        .expect("stage original token");
    assert_eq!(runtime.pane_bootstrap_session_id("%0"), Some(spawned));
    assert_eq!(runtime.bootstrap_stage_count("%0"), 1);
    assert_eq!(runtime.bootstrap_script_count("%0"), 0);
    assert!(runtime.control_pane_owns_retained_init("%0"));
    assert!(!runtime.pane_bootstrap_ready("%0"));

    gateway.on_tmux_control_mode(false);
    assert!(gateway.tmux_expected_session_id().is_none());
    runtime.unregister();
    gateway.set_tmux_instance_id(None);

    gateway.on_tmux_control_mode(true);
    let second = gateway.tmux_instance_id().expect("fresh runtime");
    assert_ne!(id, second);
    let runtime2 = TmuxRuntime::for_id(TmuxInstanceId::from_u64(second)).expect("runtime");
    assert!(runtime2.spawned_expected_session().is_none());
    assert!(runtime2.take_retained_init_send_keys("%0").is_empty());
    runtime2.unregister();
}
