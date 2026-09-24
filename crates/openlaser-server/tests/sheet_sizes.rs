// SPDX-License-Identifier: GPL-3.0-or-later
//! Saved sheet sizes belong to the machine: every screen sees them, they
//! survive a restart, and a stale save from another screen is refused.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known fixtures")]
mod common;

use openlaser_server::Coordinator;
use openlaser_server::sheet_sizes::SheetSize;

#[tokio::test]
async fn saved_sheet_sizes_are_shared_kept_and_protected_from_stale_saves() {
    let (_simulator, shared) = common::start("sheet-sizes").await;
    let size = SheetSize { width_mm: 1250., height_mm: 2500. };
    let config = {
        let mut c = shared.lock().await;
        assert!(c.document().sheet_sizes.is_empty());
        c.save_sheet_sizes(vec![size], &[]).unwrap();
        assert_eq!(c.document().sheet_sizes, vec![size]);
        // Another screen still holding the empty list cannot overwrite it.
        let stale = c.save_sheet_sizes(vec![], &[]).unwrap_err();
        assert!(stale.to_string().contains("another screen"), "{stale}");
        c.config.clone()
    };
    openlaser_server::shutdown(&shared).await.unwrap();
    let reopened = Coordinator::start(config).unwrap();
    assert_eq!(reopened.lock().await.document().sheet_sizes, vec![size]);
    openlaser_server::shutdown(&reopened).await.unwrap();
}
