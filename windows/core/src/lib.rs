//! Multi-Finder core (Windows port of `MultiFinderCore`).
//!
//! All logic lives here; the Tauri crate only wires commands and events to these functions.
//! Nothing in this crate depends on Tauri. Every long-running function takes
//! a progress callback (`&dyn Fn(&Event)`) and a [`Cancel`] flag so the caller can emit
//! events and stop the work from another thread.
//!
//! Hard rules carried over from the macOS app:
//! - nothing is deleted permanently (Recycle Bin only, see [`executor`]);
//! - every move/trash is one undo batch and undo verifies the item before moving it back;
//! - never overwrite an existing item;
//! - at least one copy of every duplicate group survives ([`executor::Executor::trash_duplicates`]).
//!
//! # Wiring (what the Tauri layer keeps and calls)
//!
//! ```text
//! data_dir      = paths::data_dir()                       (MF_DATA_DIR overrides)
//! settings      = settings::Settings::load(&data_dir)     → .view(locale, &data_dir, version)
//! store         = Arc::new(IndexStore::open_in_or_reset(&data_dir)?)
//! rules         = ScanRules::platform_default(&data_dir)
//! executor      = Executor::new(store.clone(), ProtectedPaths::platform_default(&data_dir))
//! session_start = store.max_op_id()?                      (undo only touches this session)
//! models        = model::ModelManager::new(&paths::models_dir(&data_dir))
//! lang          = classify::ui_language(&os_locale)       ("ko" | "en" | "ja" | "zh-Hans")
//!
//! startup:   store.remove_excluded(&rules); Scanner::new(rules.clone()).scan(&store, &folders, …)
//!            Watcher::start(store.clone(), rules.clone(), folders, watcher::DEFAULT_DEBOUNCE, on_change)
//! commands:  store.list_children / dir_info / folder_summaries / stats / search / folder_states
//!            executor.move_items / move_across / trash_items / trash_duplicates / undo / undo_state
//!            duplicates::find_duplicates, diff::compare_folders, classify::*, leftovers::scan_leftovers
//! ```
//! Run scans, duplicate search, compare, classification, leftovers and the model download on
//! worker threads; they block.

#![cfg_attr(test, allow(clippy::cloned_ref_to_slice_refs))]

pub mod analyzer;
pub mod classify;
pub mod diff;
pub mod duplicates;
pub mod executor;
pub mod filter;
pub mod hasher;
pub mod index;
pub mod leftovers;
#[cfg(feature = "llm")]
pub mod llm;
pub mod model;
pub mod paths;
pub mod progress;
pub mod record;
pub mod rules;
pub mod scanner;
pub mod settings;
pub mod text;
pub mod types;
pub mod watcher;

mod fsops;
#[cfg(all(windows, feature = "ocr"))]
mod ocr;

pub use executor::{Executor, UndoOutcome};
pub use index::IndexStore;
pub use paths::PathStyle;
pub use progress::{is_cancelled, Cancel, Cancelled};
pub use record::{FileKind, FileRecord};
pub use rules::{ProtectedPaths, ScanRules};
pub use scanner::{ScanOutcome, Scanner};
pub use types::*;
pub use watcher::Watcher;

#[cfg(test)]
mod thread_safety {
    fn send_sync<T: Send + Sync>() {}
    fn send<T: Send>() {}

    /// What the Tauri layer keeps in its state must be shareable across command threads.
    #[test]
    fn shared_state_is_send_and_sync() {
        send_sync::<crate::IndexStore>();
        send_sync::<crate::Executor>();
        send_sync::<crate::Scanner>();
        send_sync::<crate::model::ModelManager>();
        send_sync::<crate::Cancel>();
        send::<crate::Watcher>();
    }
}
