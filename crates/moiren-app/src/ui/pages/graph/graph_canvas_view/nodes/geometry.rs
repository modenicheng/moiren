use gpui_kit::{Path, PathBuilder, Pixels, point, px};

/// A filled pill between two points: straight sides plus semicircular
/// caps. gpui's stroke builder has no reachable line-cap option, so pill
/// ends are built as geometry.
pub(super) fn capsule_path(
    a: (f32, f32),
    b: (f32, f32),
    half: f32,
) -> anyhow::Result<Path<Pixels>> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx.hypot(dy);
    if length < 1e-3 {
        anyhow::bail!("degenerate capsule");
    }
    let (ux, uy) = (dx / length, dy / length);
    // Walk the +normal side a -> b, round the far end through the forward
    // direction (normal - 90°), come back along the -normal side, and
    // round the near end through the backward direction.
    let normal = ux.atan2(-uy).to_degrees();
    let mut builder = PathBuilder::fill();
    builder.move_to(point(px(a.0 - uy * half), px(a.1 + ux * half)));
    builder.line_to(point(px(b.0 - uy * half), px(b.1 + ux * half)));
    cap(&mut builder, b, half, normal, normal - 180.);
    builder.line_to(point(px(a.0 + uy * half), px(a.1 - ux * half)));
    cap(&mut builder, a, half, normal + 180., normal);
    builder.build()
}

/// A filled ring segment with pill ends, `half` thick either side of
/// `radius`, swept between two screen angles in degrees (y down, so 90°
/// points at the bottom).
pub(super) fn arc_capsule_path(
    center: (f32, f32),
    radius: f32,
    half: f32,
    from_deg: f32,
    to_deg: f32,
) -> anyhow::Result<Path<Pixels>> {
    let (from, to) = if to_deg >= from_deg {
        (from_deg, to_deg)
    } else {
        (to_deg, from_deg)
    };
    if to - from < 0.5 {
        anyhow::bail!("degenerate arc");
    }
    let steps = ((to - from) / 6.).ceil().max(2.) as usize;
    let on_ring = |angle_deg: f32, ring: f32| {
        let angle = angle_deg.to_radians();
        (center.0 + angle.cos() * ring, center.1 + angle.sin() * ring)
    };
    let mut builder = PathBuilder::fill();
    for step in 0..=steps {
        let angle = from + (to - from) * step as f32 / steps as f32;
        let (x, y) = on_ring(angle, radius + half);
        if step == 0 {
            builder.move_to(point(px(x), px(y)));
        } else {
            builder.line_to(point(px(x), px(y)));
        }
    }
    cap(&mut builder, on_ring(to, radius), half, to, to + 180.);
    for step in 0..=steps {
        let angle = to - (to - from) * step as f32 / steps as f32;
        let (x, y) = on_ring(angle, radius - half);
        builder.line_to(point(px(x), px(y)));
    }
    cap(
        &mut builder,
        on_ring(from, radius),
        half,
        from + 180.,
        from + 360.,
    );
    builder.build()
}

/// Half-circle cap around `center` from one screen angle to another, as
/// line segments appended to a path under construction.
fn cap(builder: &mut PathBuilder, center: (f32, f32), half: f32, from_deg: f32, to_deg: f32) {
    const STEPS: usize = 8;
    for step in 1..=STEPS {
        let angle = (from_deg + (to_deg - from_deg) * step as f32 / STEPS as f32).to_radians();
        builder.line_to(point(
            px(center.0 + angle.cos() * half),
            px(center.1 + angle.sin() * half),
        ));
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Path, Pixels};

    use super::{arc_capsule_path, capsule_path};

    /// Total area of the tessellated triangles — a self-intersecting outline
    /// (a cap bulging the wrong way) would not add up to the analytic shape.
    fn area(path: &Path<Pixels>) -> f32 {
        path.vertices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|vertices| {
                let (a, b, c) = (
                    vertices[0].xy_position,
                    vertices[1].xy_position,
                    vertices[2].xy_position,
                );
                ((f32::from(b.x - a.x) * f32::from(c.y - a.y)
                    - f32::from(c.x - a.x) * f32::from(b.y - a.y))
                    / 2.)
                    .abs()
            })
            .sum()
    }

    #[test]
    fn capsule_covers_its_body_and_two_round_caps() {
        for (a, b, half) in [
            ((10., 10.), (60., 10.), 2.),
            ((0., 0.), (30., 40.), 1.6),
            ((50., 20.), (20., 50.), 3.),
        ] {
            let path = capsule_path(a, b, half).unwrap();
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            let expected = 2. * half * length + std::f32::consts::PI * half * half;
            let actual = area(&path);
            assert!(
                (actual - expected).abs() / expected < 0.02,
                "{a:?}->{b:?}: {actual} vs {expected}"
            );
        }
        assert!(capsule_path((5., 5.), (5., 5.), 2.).is_err());
    }

    #[test]
    fn arc_capsule_covers_its_ring_segment_and_two_round_caps() {
        for (from, to) in [(135., 405.), (270., 300.), (300., 270.), (135., 270.)] {
            let (radius, half) = (17., 1.5);
            let path = arc_capsule_path((40., 40.), radius, half, from, to).unwrap();
            let sweep = (to - from).abs().to_radians();
            let expected = 2. * half * radius * sweep + std::f32::consts::PI * half * half;
            let actual = area(&path);
            assert!(
                (actual - expected).abs() / expected < 0.03,
                "{from}..{to}: {actual} vs {expected}"
            );
        }
        assert!(arc_capsule_path((0., 0.), 10., 1., 90., 90.2).is_err());
    }
}
