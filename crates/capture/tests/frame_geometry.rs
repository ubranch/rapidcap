//! Region maths and the crop it feeds.
//!
//! Every capture is a rectangle chosen with a mouse, clamped to a monitor, then
//! cut out of a padded GPU frame. A sign error anywhere in that chain is not a
//! crash - it is a screenshot of the wrong thing, which is worse, because
//! nothing reports it.

use rapidcap_capture::{PhysicalRegion, RawFrame};

fn region(x: i32, y: i32, width: u32, height: u32) -> PhysicalRegion {
    PhysicalRegion {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn a_drag_in_any_direction_gives_the_same_rectangle() {
    let expected = region(10, 20, 100, 50);
    let corners = [
        ((10, 20), (110, 70)),
        ((110, 70), (10, 20)),
        ((110, 20), (10, 70)),
        ((10, 70), (110, 20)),
    ];
    for (start, end) in corners {
        assert_eq!(
            PhysicalRegion::from_drag(start, end),
            Some(expected.clone()),
            "drag {start:?} to {end:?}"
        );
    }
}

#[test]
fn a_drag_across_the_left_monitor_keeps_its_negative_origin() {
    // A monitor left of the primary has negative virtual coordinates. Clamping
    // those to zero would silently move the capture onto the primary screen.
    assert_eq!(
        PhysicalRegion::from_drag((-1920, -100), (-1720, 100)),
        Some(region(-1920, -100, 200, 200))
    );
}

#[test]
fn a_drag_too_small_to_be_deliberate_is_not_a_capture() {
    // A click is a drag of zero pixels. Below two pixels there is no image to
    // encode, so the selection is refused rather than saved as a sliver.
    for end in [(10, 20), (11, 21), (10, 40), (40, 20)] {
        assert_eq!(
            PhysicalRegion::from_drag((10, 20), end),
            None,
            "drag to {end:?} is under the minimum in at least one axis"
        );
    }
    assert!(PhysicalRegion::from_drag((10, 20), (12, 22)).is_some());
}

#[test]
fn an_intersection_with_no_overlap_is_none() {
    let monitor = region(0, 0, 1920, 1080);
    for outside in [
        region(-500, 0, 400, 100),
        region(1920, 0, 400, 100),
        region(0, -500, 100, 400),
        region(0, 1080, 100, 400),
    ] {
        assert_eq!(
            outside.intersection(monitor.clone()),
            None,
            "{outside:?} does not touch the monitor"
        );
    }
}

#[test]
fn a_region_hanging_off_an_edge_is_clamped_to_what_the_monitor_shows() {
    let monitor = region(0, 0, 1920, 1080);
    assert_eq!(
        region(-100, -100, 400, 400).intersection(monitor.clone()),
        Some(region(0, 0, 300, 300))
    );
    assert_eq!(
        region(1820, 980, 400, 400).intersection(monitor),
        Some(region(1820, 980, 100, 100))
    );
}

#[test]
fn an_intersection_that_only_touches_an_edge_is_not_a_capture() {
    // Sharing a border means zero overlapping pixels, and a one-pixel overlap
    // is under the same two-pixel floor the drag uses.
    let monitor = region(0, 0, 1920, 1080);
    assert_eq!(
        region(-400, 0, 400, 100).intersection(monitor.clone()),
        None
    );
    assert_eq!(region(-400, 0, 401, 100).intersection(monitor), None);
}

#[test]
fn a_region_inside_the_monitor_comes_back_untouched() {
    let inner = region(100, 200, 300, 400);
    assert_eq!(
        inner.clone().intersection(region(0, 0, 1920, 1080)),
        Some(inner)
    );
}

#[test]
fn a_second_monitor_clamps_against_its_own_origin_not_the_desktop() {
    // The right-hand monitor starts at x = 1920, so a drag that begins on the
    // primary must not drag pixels in from the neighbour.
    let right = region(1920, 0, 2560, 1440);
    assert_eq!(
        region(1800, 100, 400, 200).intersection(right),
        Some(region(1920, 100, 280, 200))
    );
}

fn padded_frame(width: u32, height: u32, stride: u32) -> RawFrame {
    // Distinct BGRA per pixel so a wrong offset shows up as wrong colours
    // rather than as a plausible-looking image.
    let mut bytes = vec![0_u8; stride as usize * height as usize];
    for y in 0..height {
        for x in 0..width {
            let index = y as usize * stride as usize + x as usize * 4;
            bytes[index] = x as u8;
            bytes[index + 1] = y as u8;
            bytes[index + 2] = (x + y) as u8;
            bytes[index + 3] = 255;
        }
    }
    RawFrame {
        bytes,
        width,
        height,
        stride,
    }
}

#[test]
fn a_crop_reads_past_the_row_padding_not_through_it() {
    // The GPU hands back rows padded to a stride wider than the image. Reading
    // width*4 per row instead of the stride slides every row left by the
    // padding and skews the whole picture.
    let frame = padded_frame(8, 4, 8 * 4 + 16);
    let cropped = frame.crop_rgba(region(2, 1, 4, 2)).unwrap();

    assert_eq!(cropped.width, 4);
    assert_eq!(cropped.height, 2);
    // Top-left of the crop is source pixel (2, 1): B=2, G=1, R=3 becomes RGBA.
    assert_eq!(&cropped.rgba[0..4], &[3, 1, 2, 255]);
    // Second row, first pixel is source (2, 2).
    let second_row = 4 * 4;
    assert_eq!(&cropped.rgba[second_row..second_row + 4], &[4, 2, 2, 255]);
}

#[test]
fn a_crop_of_the_whole_frame_keeps_every_pixel() {
    let frame = padded_frame(6, 3, 6 * 4);
    let cropped = frame.crop_rgba(region(0, 0, 6, 3)).unwrap();
    assert_eq!(cropped.rgba.len(), 6 * 3 * 4);
    assert_eq!(cropped.width, 6);
    assert_eq!(cropped.height, 3);
}

#[test]
fn a_crop_outside_the_frame_is_refused_rather_than_read() {
    let frame = padded_frame(8, 4, 8 * 4);
    for bad in [
        region(-1, 0, 4, 2),
        region(0, -1, 4, 2),
        region(6, 0, 4, 2),
        region(0, 3, 4, 2),
        region(0, 0, 9, 4),
        region(0, 0, 8, 5),
    ] {
        assert!(
            frame.crop_rgba(bad.clone()).is_none(),
            "{bad:?} reaches outside the frame"
        );
    }
}

#[test]
fn a_crop_of_a_truncated_buffer_is_refused() {
    // A short buffer means the capture never finished arriving. Cropping it
    // would index past the end.
    let mut frame = padded_frame(8, 4, 8 * 4);
    frame.bytes.truncate(8 * 4 * 3);
    assert!(frame.crop_rgba(region(0, 0, 8, 4)).is_none());
}

#[test]
fn a_frame_claiming_a_stride_narrower_than_its_width_is_refused() {
    let mut frame = padded_frame(8, 4, 8 * 4);
    frame.stride = 8 * 4 - 4;
    assert!(frame.crop_rgba(region(0, 0, 8, 4)).is_none());
}
