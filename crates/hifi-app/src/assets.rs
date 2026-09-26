//! Embedded assets: the Solar Icons (Linear weight) set — the same icon
//! library cosmos ships — plus Geist / Geist Mono fonts.

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! icon_assets {
    ($(($const_name:ident, $path:literal)),+ $(,)?) => {
        $(pub const $const_name: &str = concat!("icons/", $path, ".svg");)+

        pub const ICON_PATHS: &[&str] = &[$(concat!("icons/", $path, ".svg")),+];

        pub fn load_icon(path: &str) -> Option<Cow<'static, [u8]>> {
            match path {
                $(concat!("icons/", $path, ".svg") => Some(Cow::Borrowed(
                    include_bytes!(concat!("../assets/icons/", $path, ".svg")).as_slice(),
                )),)+
                _ => None,
            }
        }
    };
}

pub mod icons {
    // The full Solar set is bundled for callers to reach by name; only a
    // subset is referenced today.
    #![allow(dead_code)]
    use super::Cow;

    icon_assets![
        (ADD_CIRCLE, "add-circle"),
        (ALT_ARROW_DOWN, "alt-arrow-down"),
        (ALT_ARROW_LEFT, "alt-arrow-left"),
        (ALT_ARROW_RIGHT, "alt-arrow-right"),
        (ALT_ARROW_UP, "alt-arrow-up"),
        (AMP_MARK, "amp-mark"),
        (ANTIGRAVITY_MARK, "antigravity-mark"),
        (ARCHIVE_MINIMALISTIC, "archive-minimalistic"),
        (ARCHIVE_UP, "archive-up-minimalistic"),
        (ARROW_DOWN, "arrow-down"),
        (ARROW_LEFT, "arrow-left"),
        (ARROW_RIGHT, "arrow-right"),
        (ARROW_UP_LEFT, "arrow-up-left"),
        (ARROW_UP_RIGHT, "arrow-up-right"),
        (ARROW_UP, "arrow-up"),
        (BELL, "bell"),
        (BOLT, "bolt"),
        (BOT, "bot"),
        (CALENDAR, "calendar"),
        (CAMERA_VIEWFINDER, "camera-viewfinder"),
        (CAMERA, "camera"),
        (CHAT_ROUND_LINE, "chat-round-line"),
        (CHECK_CIRCLE, "check-circle"),
        (CHECK, "check"),
        (CHECKLIST, "checklist"),
        (CIRCLE_HALF, "circle-half"),
        (CLAUDE_MARK, "claude-mark"),
        (CLOCK_CIRCLE, "clock-circle"),
        (CLOSE_CIRCLE, "close-circle"),
        (CLOSE, "close"),
        (CLOUD, "cloud"),
        (COLLAPSE_ARROWS, "collapse-arrows"),
        (COMMAND, "command"),
        (COPY, "copy"),
        (CURSOR_MARK, "cursor-mark"),
        (DANGER_TRIANGLE, "danger-triangle"),
        (DEVIN_MARK, "devin-mark"),
        (DOCUMENT_ADD, "document-add"),
        (DOCUMENT, "document"),
        (DRAG_HANDLE, "drag-handle"),
        (ELLIPSIS, "ellipsis"),
        (EXPAND_ARROWS, "expand-arrows"),
        (EYE_CLOSED, "eye-closed"),
        (EYE, "eye"),
        (FILE_CODE, "file-code"),
        (FILE_DATA, "file-data"),
        (FILE_IMAGE, "file-image"),
        (FILE_MARKDOWN, "file-markdown"),
        (FILE_STYLE, "file-style"),
        (FLOPPY_DISK, "floppy-disk"),
        (FOLD_VERTICAL, "fold-vertical"),
        (FOLDER_WITH_FILES, "folder-with-files"),
        (FOLDER, "folder"),
        (GAUGE, "gauge"),
        (GIT_BRANCH, "git-branch"),
        (GLOBAL, "global"),
        (GLOBE, "globe"),
        (GROK_MARK, "grok-mark"),
        (HAND, "hand"),
        (HARD_DRIVE, "hard-drive"),
        (HERMES_MARK, "hermes-mark"),
        (HOME, "home"),
        (INFO_CIRCLE, "info-circle"),
        (KEY_MINIMALISTIC, "key-minimalistic"),
        (KEYBOARD, "keyboard"),
        (KIMI_MARK, "kimi-mark"),
        (LAPTOP, "laptop"),
        (LINK, "link"),
        (LIST, "list"),
        (LOCK, "lock"),
        (LOGOUT, "logout-2"),
        (MAGNIFER, "magnifer"),
        (MICROPHONE, "microphone"),
        (MONITOR, "monitor"),
        (MOON, "moon"),
        (OPENAI_MARK, "openai-mark"),
        (OPENCODE_MARK, "opencode-mark"),
        (PALETTE_SEARCH, "palette-search"),
        (PAPERCLIP, "paperclip"),
        (PAUSE, "pause"),
        (PEN_NEW_SQUARE, "pen-new-square"),
        (PEN, "pen"),
        (PHOTO, "photo"),
        (PI_MARK, "pi-mark"),
        (PIN, "pin"),
        (PLAY, "play"),
        (PLUS, "plus"),
        (PROJECT_DEFAULT, "project-default"),
        (PULL_REQUEST, "pull-request"),
        (QR_CODE, "qr-code"),
        (QUEUE_CHECK, "queue-check"),
        (QUEUE_CLOSE, "queue-close"),
        (QUEUE_DRAG_HANDLE, "queue-drag-handle"),
        (QUEUE_PAPERCLIP, "queue-paperclip"),
        (QUEUE_SEND, "queue-send"),
        (RECORD_CIRCLE, "record-circle"),
        (REFRESH, "refresh"),
        (REMOTE_SERVER, "remote-server"),
        (RESTART, "restart"),
        (RETURN, "return"),
        (SEND, "send"),
        (SETTINGS, "settings-minimalistic"),
        (SHIELD, "shield"),
        (SIDEBAR_LEFT, "sidebar-minimalistic-left"),
        (SIDEBAR, "sidebar-minimalistic"),
        (SMARTPHONE, "smartphone"),
        (SORT_VERTICAL, "sort-vertical"),
        (SORT, "sort"),
        (SPLIT_COLUMNS, "split-columns"),
        (STAR, "star-bold"),
        (STOP, "stop"),
        (SUN, "sun"),
        (TAG, "tag"),
        (TERMINAL, "terminal"),
        (TRASH, "trash-bin-minimalistic"),
        (TUNING, "tuning"),
        (USER_CIRCLE, "user-circle"),
        (VOLUME_LOUD, "volume-loud"),
        (WIDGET, "widget"),
        (WIFI_OFF, "wifi-off"),
        (WIFI, "wifi"),
        (WINDOW_MAXIMIZE, "window-maximize"),
        (WINDOW_MINIMIZE, "window-minimize"),
        (WINDOW_RESTORE, "window-restore"),
        (WRAP_TEXT, "wrap-text"),
        (ZERON_LOGO, "zeron-logo"),
    ];
}

pub struct Assets;

pub const FONTS: &[&str] = &[
    "fonts/Geist.ttf",
    "fonts/Geist-Bold.ttf",
    "fonts/Geist-BoldItalic.ttf",
    "fonts/Geist-Italic.ttf",
    "fonts/Geist-Medium.ttf",
    "fonts/Geist-MediumItalic.ttf",
    "fonts/Geist-SemiBold.ttf",
    "fonts/Geist-SemiBoldItalic.ttf",
    "fonts/GeistMono.ttf",
    "fonts/GeistMono-Bold.ttf",
    "fonts/GeistMono-BoldItalic.ttf",
    "fonts/GeistMono-Italic.ttf",
    "fonts/GeistMono-Medium.ttf",
    "fonts/GeistMono-MediumItalic.ttf",
    "fonts/GeistMono-SemiBold.ttf",
    "fonts/GeistMono-SemiBoldItalic.ttf",
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = icons::load_icon(path) {
            return Ok(Some(icon));
        }
        Ok(match path {
            p if FONTS.contains(&p) => Some(Cow::Borrowed(match p {
                "fonts/Geist.ttf" => include_bytes!("../assets/fonts/Geist.ttf").as_slice(),
                "fonts/Geist-Bold.ttf" => include_bytes!("../assets/fonts/Geist-Bold.ttf").as_slice(),
                "fonts/Geist-BoldItalic.ttf" => {
                    include_bytes!("../assets/fonts/Geist-BoldItalic.ttf").as_slice()
                }
                "fonts/Geist-Italic.ttf" => {
                    include_bytes!("../assets/fonts/Geist-Italic.ttf").as_slice()
                }
                "fonts/Geist-Medium.ttf" => {
                    include_bytes!("../assets/fonts/Geist-Medium.ttf").as_slice()
                }
                "fonts/Geist-MediumItalic.ttf" => {
                    include_bytes!("../assets/fonts/Geist-MediumItalic.ttf").as_slice()
                }
                "fonts/Geist-SemiBold.ttf" => {
                    include_bytes!("../assets/fonts/Geist-SemiBold.ttf").as_slice()
                }
                "fonts/GeistMono.ttf" => include_bytes!("../assets/fonts/GeistMono.ttf").as_slice(),
                "fonts/GeistMono-Bold.ttf" => {
                    include_bytes!("../assets/fonts/GeistMono-Bold.ttf").as_slice()
                }
                "fonts/GeistMono-BoldItalic.ttf" => {
                    include_bytes!("../assets/fonts/GeistMono-BoldItalic.ttf").as_slice()
                }
                "fonts/GeistMono-Italic.ttf" => {
                    include_bytes!("../assets/fonts/GeistMono-Italic.ttf").as_slice()
                }
                "fonts/GeistMono-Medium.ttf" => {
                    include_bytes!("../assets/fonts/GeistMono-Medium.ttf").as_slice()
                }
                "fonts/GeistMono-MediumItalic.ttf" => {
                    include_bytes!("../assets/fonts/GeistMono-MediumItalic.ttf").as_slice()
                }
                "fonts/GeistMono-SemiBold.ttf" => {
                    include_bytes!("../assets/fonts/GeistMono-SemiBold.ttf").as_slice()
                }
                "fonts/GeistMono-SemiBoldItalic.ttf" => {
                    include_bytes!("../assets/fonts/GeistMono-SemiBoldItalic.ttf").as_slice()
                }
                _ => return Ok(None),
            })),
            _ => None,
        })
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all: Vec<SharedString> = icons::ICON_PATHS
            .iter()
            .chain(FONTS.iter())
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(*p))
            .collect();
        all.sort();
        Ok(all)
    }
}
