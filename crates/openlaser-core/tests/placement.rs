// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared placement rules protect both edits and retained layouts.

use openlaser_core::geometry::{MAX_PLACED_CONTOURS, Placed, Transform};

#[test]
fn source_transform_size_and_instance_identity_share_one_validation_boundary() {
    let valid = [Placed::drawn(0), Placed { source: 0, copy: 1, transform: Transform::IDENTITY }];
    assert!(Placed::validate_all(&valid, 1).is_ok());
    assert!(Placed::validate_all(&[], 1).is_err());
    assert!(Placed::validate_all(&[Placed::drawn(1)], 1).is_err());
    assert!(Placed::validate_all(&[Placed::drawn(0), Placed::drawn(0)], 1).is_err());
    assert!(
        Placed::validate_all(&[Placed { source: 0, copy: 0, transform: Transform([0.; 6]) }], 1)
            .is_err()
    );
    assert!(Placed::validate_all(&vec![Placed::drawn(0); MAX_PLACED_CONTOURS + 1], 1).is_err());
}
