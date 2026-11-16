//! # What is accessibility?
//! Accessibility (or a11y) is the umbrella term used to describe features that enable people with
//! disabilities to use certain software. In our case: we focus on blind users and their day-to-day
//! life with screen readers.
//!
//! ## How does a11y work in Warp?
//! Because Warp uses its own rust UI framework (warpui), we don’t benefit from the built-in
//! VoiceOver integration and objc NSAccessibility APIs. This is both good and bad for our app and
//! the UI framework.
//!
//! Good parts:
//! - We actually had to think on how to add support to the UI framework to make it easier for future
//!   app developers to not overlook a11y;
//! - We don’t rely on complicated defaults and the cumbersome experience of analyzing all the UI
//!   elements in the app, instead we can provide a more ergonomic experience to blind users.
//!
//! Bad parts:
//! - It takes time to implement the full support (for example, we still lack the ability to focus a
//!   certain UI element, like a button, and “click it” or otherwise act on it with just the keyboard);
//! - We need to think about a11y (yeah, I mentioned it in good parts, but given that a11y is usually
//!   thirdwheeling next to AwesomeFeatures™ and BugFixes® it’s easy to ship features that are not accessible).
//!
//! WarpUI framework right now provides 3 ways of announcing what’s happening in the app:
//! - Accessibility Contents for the currently focused View;
//! - Accessibility Contents for the currently performed Action;
//! - On-demand emitting Accessibility Contents.
//!
//! ## Testing for a11y
//! We don’t have (and I don’t know if such a thing even exists) a way to automatically test a11y
//! features. To test it then, we just need to run the app and run VoiceOver.
//!
//! To run it - go to your System Preferences -> Accessibility -> VoiceOver, and then click
//! “Enable VoiceOver”. Note that it may be loud and distracting. It’s sometimes easier to turn off
//! the sound, and check the content of the tiny rectangle that will show on your screen together
//! with VoiceOver.
//!
//! ### What to look for?
//! - Whenever a new view opens, the user will get the information about what’s happening;
//! - The feature is keyboard accessible (a good practice would be to have it in the command palette);
//! - Any meaningful changes to the state of feature are announced (both triggered by a user’s
//!   Action or a background Event);
//! - The user can quit the feature and get back to the command input with keyboard (a good
//!   practice would be to keep it consistent among all the features, and quit via Escape key);
//! - User docs mention whether the feature is accessible (on the feature’s page) and what’s the
//!   keybinding to access it;
//! - If there’s a video/GIF in the user docs, make sure that its content is also reflected in text.

use pathfinder_geometry::rect::RectF;
use serde::{Deserialize, Serialize};

use crate::Action;

#[derive(Debug, Clone)]
/// Main structure describing the content VoiceOver (or other screen reading software) will receive.
pub struct AccessibilityContent {
    /// The main information related to the view/action/event. Keep it short and as informative
    /// as possible. It’s semi-equivalent to
    /// [AccessibilityLabel](https://developer.apple.com/documentation/appkit/nsaccessibility/1534976-accessibilitylabel).
    /// For example, a `value` for the command editor in our case is “Command Input”.
    pub value: String,
    /// Optional string that provides more context and information about available actions.
    /// For example, help for the “Command Input” informs about “cmd-up” action.
    pub help: Option<String>,
    /// (currently unused) The rectangle that describes where the given element is on the screen.
    /// System’s APIs then draw a frame around that element, making it super clear what object
    /// the description is referring to.
    /// Frame support is a work-in-progress in Warp and right now this field is omitted and not set.
    pub frame: Option<RectF>,
    /// The role a given element has. Note that we use our own, WarpUI-defined roles (vs those that
    /// come from the NSAccessibility framework). The role describes the action/element/event role (
    /// for example, when the “Command Input” is focused, it announces with a `TextareaRole`.
    /// This is another helper field that lets the user understand what they can potentially do,
    /// or what object is in focus.
    pub role: WarpA11yRole,
}

/// Verbosity level of a11y announcements. By default, all announcements include both the value
/// and help (if provided). It can be changed per-app basis, in AppContext.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema_gen", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema_gen",
    schemars(
        description = "Verbosity level for screen reader announcements.",
        rename_all = "snake_case"
    )
)]
#[cfg_attr(feature = "settings_value", derive(settings_value::SettingsValue))]
pub enum AccessibilityVerbosity {
    /// Default verbosity level, includes help string.
    #[default]
    #[serde(rename = "VERBOSE")]
    #[cfg_attr(feature = "schema_gen", schemars(rename = "verbose"))]
    Verbose,
    /// Concise level, only announces `value` from AccessibilityContent.
    #[serde(rename = "CONCISE")]
    #[cfg_attr(feature = "schema_gen", schemars(rename = "concise"))]
    Concise,
}

/// For single character strings, we want to announce extra information (such as
/// capitalization). For longer/shorter strings - we just return the string itself.
// Note: This should be localized or passed directly to voice over in a "working" format.
// For some reason, VO currently ignores out punctuation and capital letters so we need to do it
// manually. The Appkit Obj-C APIs don't provide support to enforce punctuation or change of pitch
// for capital letters, so for now we're just implementing the missing pieces by hand, yay!
fn string_announcement(s: String) -> String {
    if s.len() != 1 {
        return s;
    }

    let c = s.chars().next().expect("String has exactly 1 character");

    if c.is_uppercase() {
        return format!("capital {s}");
    }

    if c.is_ascii_punctuation() {
        return match c {
            '.' => "period".to_string(),
            '!' => "exclamation mark".to_string(),
            '~' => "tilde".to_string(),
            '`' => "accent".to_string(),
            '^' => "caret".to_string(),
            '(' => "left parenthesis".to_string(),
            ')' => "right parenthesis".to_string(),
            '-' => "hyphen".to_string(),
            '_' => "underscore".to_string(),
            '?' => "question mark".to_string(),
            ':' => "colon".to_string(),
            ';' => "semicolon".to_string(),
            '"' => "double quotation mark".to_string(),
            '\'' => "single quotation mark".to_string(),
            '\\' => "backslash".to_string(),
            '/' => "slash".to_string(),
            ',' => "comma".to_string(),
            '[' => "left bracket".to_string(),
            ']' => "right bracket".to_string(),
            '{' => "left brace".to_string(),
            '}' => "right brace".to_string(),
            '|' => "vertical line".to_string(),

            // everything else seems to have proper interpretation in voiceover
            _ => s,
        };
    }
    s
}

impl AccessibilityContent {
    // TODO add frame support
    pub fn new_without_help<T>(value: T, role: WarpA11yRole) -> Self
    where
        T: Into<String>,
    {
        Self::new_internal::<T, String>(value, None, role)
    }

    pub fn new<V, H>(value: V, help: H, role: WarpA11yRole) -> Self
    where
        V: Into<String>,
        H: Into<String>,
    {
        Self::new_internal(value, Some(help), role)
    }

    fn new_internal<V, H>(value: V, help: Option<H>, role: WarpA11yRole) -> Self
    where
        V: Into<String>,
        H: Into<String>,
    {
        let value: String = value.into();
        // Note that for values that are all whitespace, we still want to read them out, hence
        // swapping certain whitespace characters with their "readings".
        let value = if value.chars().all(char::is_whitespace) {
            value
                .replace(' ', " space ") // Note: order here is important, space should go first.
                .replace('\t', " tab ")
                .replace('\n', " newline ")
                .trim()
                .to_string()
        } else {
            string_announcement(value)
        };
        AccessibilityContent {
            value,
            help: help.map(|s| s.into()),
            role,
            frame: None,
        }
    }

    pub fn with_frame(mut self, frame: Option<RectF>) -> Self {
        self.frame = frame;
        self
    }

    pub fn with_verbosity(mut self, verbosity: AccessibilityVerbosity) -> Self {
        if matches!(verbosity, AccessibilityVerbosity::Concise) {
            self.help = None;
        }
        self
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub enum WarpA11yRole {
    ButtonRole,
    CheckboxRole,
    HelpRole,
    ImageRole,
    LinkRole,
    ListRole,
    MenuItemRole,
    MenuRole,
    PopoverRole,
    ScrollareaRole,
    TextRole,
    TextareaRole,
    TextfieldRole,
    #[default]
    WindowRole,
    UserAction,
}

impl std::fmt::Display for WarpA11yRole {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use WarpA11yRole::*;
        let word = match self {
            ButtonRole => "Button",
            CheckboxRole => "Checkbox",
            HelpRole => "Help",
            ImageRole => "Image",
            LinkRole => "Link",
            ListRole => "List",
            MenuItemRole => "MenuItem",
            MenuRole => "Menu",
            PopoverRole => "Popover",
            ScrollareaRole => "Scrollarea",
            TextRole => "Text",
            TextareaRole => "Textarea",
            TextfieldRole => "Textfield",
            WindowRole => "Window",
            UserAction => "Action",
        };
        write!(f, "{word}")
    }
}

#[derive(Default)]
pub enum ActionAccessibilityContent {
    #[default]
    Empty,
    Custom(AccessibilityContent),
    CustomFn(fn(&dyn Action) -> AccessibilityContent),
}

impl ActionAccessibilityContent {
    pub fn from_debug() -> Self {
        Self::CustomFn(|action| {
            AccessibilityContent::new_without_help(format!("{action:?}."), WarpA11yRole::UserAction)
        })
    }
}

impl From<Option<AccessibilityContent>> for ActionAccessibilityContent {
    fn from(opt: Option<AccessibilityContent>) -> ActionAccessibilityContent {
        match opt {
            None => ActionAccessibilityContent::Empty,
            Some(content) => ActionAccessibilityContent::Custom(content),
        }
    }
}
