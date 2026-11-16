pub(crate) fn x_axis_badge_left(
    plot_x: f32,
    plot_width: f32,
    anchor_x: f32,
    label_width: f32,
) -> Option<f32> {
    if !plot_x.is_finite()
        || !plot_width.is_finite()
        || !anchor_x.is_finite()
        || !label_width.is_finite()
        || label_width <= 0.0
        || plot_width < label_width
    {
        return None;
    }

    let max_x = plot_x + plot_width - label_width;
    if !max_x.is_finite() || max_x < plot_x {
        return None;
    }

    Some((anchor_x - label_width / 2.0).clamp(plot_x, max_x))
}

#[cfg(test)]
mod tests {
    use super::x_axis_badge_left;

    #[test]
    fn skips_badge_when_plot_is_narrower_than_label() {
        assert_eq!(x_axis_badge_left(0.0, 90.92, 45.0, 100.0), None);
    }

    #[test]
    fn places_badge_at_left_edge_when_plot_exactly_fits() {
        assert_eq!(x_axis_badge_left(12.0, 100.0, 60.0, 100.0), Some(12.0));
    }

    #[test]
    fn clamps_badge_within_wide_plot() {
        assert_eq!(x_axis_badge_left(10.0, 200.0, -100.0, 100.0), Some(10.0));
        assert_eq!(x_axis_badge_left(10.0, 200.0, 300.0, 100.0), Some(110.0));
    }

    #[test]
    fn skips_badge_for_non_finite_plot_geometry() {
        assert_eq!(x_axis_badge_left(f32::NAN, 200.0, 100.0, 100.0), None);
        assert_eq!(x_axis_badge_left(0.0, f32::INFINITY, 100.0, 100.0), None);
    }
}
