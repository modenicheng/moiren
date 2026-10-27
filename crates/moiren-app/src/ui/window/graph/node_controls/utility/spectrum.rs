//! Prepared FFT settings without fabricated spectrum observations.
use super::*;
use moiren_engine::spectrum::SpectrumSettings;

pub(super) fn content(
    id: NodeId,
    settings: SpectrumSettings,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut fft = div()
        .flex()
        .items_center()
        .gap(px(3.))
        .child(note("FFT").w(px(28.)).flex_none());
    for size in [512, 1024, 2048, 4096, 8192] {
        fft = fft.child(choice(
            format!("card-spectrum-fft-{id:?}-{size}"),
            size.to_string(),
            size == settings.fft_size,
            spectrum_with_fft(settings, size)
                .map(|settings| builtin_edit(id, BuiltinState::SpectrumAnalyzer(settings))),
            cx,
        ));
    }
    let mut hop = div()
        .flex()
        .items_center()
        .gap(px(3.))
        .child(note("Hop").w(px(28.)).flex_none());
    for frames in [
        settings.fft_size / 4,
        settings.fft_size / 2,
        settings.fft_size,
    ] {
        let next = SpectrumSettings {
            hop_frames: frames,
            ..settings
        };
        hop = hop.child(choice(
            format!("card-spectrum-hop-{id:?}-{frames}"),
            frames.to_string(),
            frames == settings.hop_frames,
            next.valid()
                .then(|| builtin_edit(id, BuiltinState::SpectrumAnalyzer(next))),
            cx,
        ));
    }
    // StudioShell currently has level subscriptions only. A magnitude plot
    // must wait for a real spectrum subscription; levels cannot stand in for it.
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .child(detail_line(
            "FFT / hop",
            &t!(
                "%{fft} / %{hop} frames",
                fft = settings.fft_size,
                hop = settings.hop_frames
            ),
        ))
        .child(fft)
        .child(hop)
        .child(
            div()
                .h(px(68.))
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .bg(theme::WINDOW_BG)
                .border_1()
                .border_color(theme::BORDER)
                .rounded(px(theme::RADIUS_SM))
                .child(note("Spectrum unavailable").text_color(theme::TEXT))
                .child(note("No spectrum subscription in this view")),
        )
        .into_any_element()
}

fn spectrum_with_fft(settings: SpectrumSettings, fft_size: usize) -> Option<SpectrumSettings> {
    let next = SpectrumSettings {
        fft_size,
        hop_frames: settings.hop_frames.min(fft_size),
    };
    next.valid().then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn spectrum_choices_preserve_hop_until_the_new_fft_requires_a_smaller_one() {
        let settings = SpectrumSettings {
            fft_size: 4096,
            hop_frames: 2048,
        };
        for fft_size in [512, 1024, 2048, 4096, 8192] {
            let next = spectrum_with_fft(settings, fft_size).unwrap();
            assert!(next.valid());
            assert_eq!(next.hop_frames, settings.hop_frames.min(fft_size));
        }
        for fft_size in [0, 15, 1000, 16384] {
            assert!(spectrum_with_fft(settings, fft_size).is_none());
        }
    }
}
