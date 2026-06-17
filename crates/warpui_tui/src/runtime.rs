use std::cell::Cell;
use std::io::{self, stdout, Stdout};
use std::rc::Rc;
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use warpui_core::{App, TuiViewHandle, WindowId};

use crate::elements::TuiElement;
use crate::{
    crossterm_event_to_warp_event, TuiFrame, TuiFrameRenderer, TuiPresenter, TuiSize, TuiView,
};

pub struct TuiRuntime<T> {
    window_id: WindowId,
    root_view: TuiViewHandle<T>,
    presenter: TuiPresenter,
    terminal: TuiTerminalSession,
    dirty: Rc<Cell<bool>>,
    last_size: Option<TuiSize>,
}

impl<T> TuiRuntime<T>
where
    T: TuiView<RenderOutput = Box<dyn TuiElement>>,
{
    pub fn enter(app: &App, window_id: WindowId, root_view: TuiViewHandle<T>) -> io::Result<Self> {
        let dirty = Rc::new(Cell::new(true));
        let dirty_for_invalidation = dirty.clone();
        app.on_window_invalidated(window_id, move |_, _| {
            dirty_for_invalidation.set(true);
        });

        Ok(Self {
            window_id,
            root_view,
            presenter: TuiPresenter::new(),
            terminal: TuiTerminalSession::enter()?,
            dirty,
            last_size: None,
        })
    }

    pub fn run_until(
        &mut self,
        app: &mut App,
        mut should_quit: impl FnMut(&App) -> bool,
    ) -> io::Result<()> {
        while !should_quit(app) {
            self.draw_if_dirty(app)?;
            self.poll_and_dispatch_event(app, Duration::from_millis(250))?;
        }
        Ok(())
    }

    fn draw_if_dirty(&mut self, app: &mut App) -> io::Result<()> {
        let size = self.terminal.size()?;
        if self.last_size != Some(size) {
            self.dirty.set(true);
        }

        if !self.dirty.replace(false) {
            return Ok(());
        }

        let _ = app.take_window_invalidations(self.window_id);
        let frame = app.read(|ctx| {
            self.root_view.read(ctx, |view, ctx| {
                self.presenter.render_window(
                    self.window_id,
                    self.root_view.id(),
                    view,
                    ctx,
                    size,
                )
            })
        });
        self.presenter.sync_focus(app);
        self.terminal.draw(&frame)?;
        self.last_size = Some(size);
        Ok(())
    }

    fn poll_and_dispatch_event(&mut self, app: &mut App, timeout: Duration) -> io::Result<()> {
        if !event::poll(timeout)? {
            return Ok(());
        }

        match event::read()? {
            CrosstermEvent::Resize(_, _) => {
                self.dirty.set(true);
            }
            event => {
                if let Some(event) = crossterm_event_to_warp_event(event) {
                    let result = self.presenter.dispatch_event(&event, app);
                    if result.handled {
                        self.dirty.set(true);
                    }
                }
            }
        }

        Ok(())
    }
}

struct TuiTerminalSession {
    stdout: Stdout,
    frame_renderer: TuiFrameRenderer,
}

impl TuiTerminalSession {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;

        let mut stdout = stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }

        Ok(Self {
            stdout,
            frame_renderer: TuiFrameRenderer::new(),
        })
    }

    fn size(&self) -> io::Result<TuiSize> {
        let (width, height) = terminal::size()?;
        Ok(TuiSize::new(width.max(1), height.max(1)))
    }

    fn draw(&mut self, frame: &TuiFrame) -> io::Result<()> {
        self.frame_renderer.draw(&mut self.stdout, frame)
    }
}

impl Drop for TuiTerminalSession {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, Show, DisableMouseCapture, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}
