use std::marker::PhantomData;
use std::sync::LazyLock;

use warpui::keymap::Keystroke;
use warpui::AppContext;

use crate::editor::{SELECT_DOWN_ACTION_NAME, SELECT_UP_ACTION_NAME};
use crate::terminal::input::inline_menu::{
    InlineMenuAction, InlineMenuMessageArgs, InlineMenuRowAction,
};
use crate::terminal::input::message_bar::{Message, MessageItem, MessageProvider};
use crate::util::bindings::keybinding_name_to_keystroke;

/// Generic message provider for inline menus.
///
/// The message line is what contains the up/down navigation hints and optionally,
/// contextual CTAs based on the currently selected item.
pub struct InlineMenuMessageProvider<A>(PhantomData<A>);

impl<A> Default for InlineMenuMessageProvider<A> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<A, T> MessageProvider<InlineMenuMessageArgs<'_, A, T>> for InlineMenuMessageProvider<A>
where
    A: InlineMenuAction,
{
    fn produce_message(&self, args: InlineMenuMessageArgs<'_, A, T>) -> Option<Message> {
        A::produce_inline_menu_message(args)
    }
}

/// Returns the default set of navigation/dismiss message items that should be rendered in all
/// inline menus.
pub fn default_navigation_message_items<A: InlineMenuAction, T>(
    args: &InlineMenuMessageArgs<'_, A, T>,
) -> Vec<MessageItem> {
    let navigation_keystrokes = navigation_keystrokes(args.app);
    let mut items = vec![
        MessageItem::keystroke(navigation_keystrokes.0),
        MessageItem::keystroke(navigation_keystrokes.1),
        MessageItem::text(" to navigate"),
    ];

    if args.inline_menu_model.tab_configs().len() > 1 {
        items.push(MessageItem::keystroke(Keystroke {
            key: "tab".to_owned(),
            shift: true,
            ..Default::default()
        }));
        items.push(MessageItem::text(" to cycle tabs"));
    }

    items.push(MessageItem::clickable(
        vec![
            MessageItem::keystroke(Keystroke {
                key: "escape".to_owned(),
                ..Default::default()
            }),
            MessageItem::text(" to dismiss"),
        ],
        |ctx| {
            ctx.dispatch_typed_action(InlineMenuRowAction::<A>::Dismiss);
        },
        args.inline_menu_model.mouse_states().dismiss.clone(),
    ));

    items
}

fn navigation_keystrokes(app: &AppContext) -> (Keystroke, Keystroke) {
    static DEFAULT_SELECT_UP_BINDING: LazyLock<Option<Keystroke>> = LazyLock::new(|| {
        if cfg!(target_os = "macos") {
            Some(Keystroke {
                key: "P".to_owned(),
                shift: true,
                ctrl: true,
                ..Default::default()
            })
        } else {
            None
        }
    });
    static DEFAULT_SELECT_DOWN_BINDING: LazyLock<Option<Keystroke>> = LazyLock::new(|| {
        if cfg!(target_os = "macos") {
            Some(Keystroke {
                key: "N".to_owned(),
                shift: true,
                ctrl: true,
                ..Default::default()
            })
        } else {
            None
        }
    });

    if let Some((up_keystroke, down_keystroke)) =
        keybinding_name_to_keystroke(SELECT_UP_ACTION_NAME, app)
            .zip(keybinding_name_to_keystroke(SELECT_DOWN_ACTION_NAME, app))
            .filter(|(up_binding, down_binding)| {
                Some(up_binding) != DEFAULT_SELECT_UP_BINDING.as_ref()
                    && Some(down_binding) != DEFAULT_SELECT_DOWN_BINDING.as_ref()
            })
    {
        (up_keystroke, down_keystroke)
    } else {
        (
            Keystroke {
                key: "up".to_owned(),
                ..Default::default()
            },
            Keystroke {
                key: "down".to_owned(),
                ..Default::default()
            },
        )
    }
}
