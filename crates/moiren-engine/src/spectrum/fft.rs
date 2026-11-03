/// Reusable radix-2 decimation-in-time workspace and precomputed forward twiddles.
pub(super) struct Radix2Fft {
    pub(super) re: Box<[f64]>,
    pub(super) im: Box<[f64]>,
    bit_reversal: Box<[usize]>,
    twiddle_re: Box<[f64]>,
    twiddle_im: Box<[f64]>,
    size: usize,
}

impl Radix2Fft {
    pub(super) fn new(size: usize) -> Self {
        let bits = size.trailing_zeros();
        let bit_reversal = (0..size)
            .map(|index| index.reverse_bits() >> (usize::BITS - bits))
            .collect();
        let half = size / 2;
        let twiddle_re = (0..half)
            .map(|index| (std::f64::consts::TAU * index as f64 / size as f64).cos())
            .collect();
        let twiddle_im = (0..half)
            .map(|index| -(std::f64::consts::TAU * index as f64 / size as f64).sin())
            .collect();
        Self {
            re: vec![0.0; size].into_boxed_slice(),
            im: vec![0.0; size].into_boxed_slice(),
            bit_reversal,
            twiddle_re,
            twiddle_im,
            size,
        }
    }

    /// Forward transform of one time-ordered window, `samples[i] * window[i]`
    /// for `i` in `0..size`; the result stays in `re`/`im`.
    pub(super) fn transform(&mut self, samples: &[f32], window: &[f64]) {
        debug_assert_eq!(samples.len(), self.size);
        debug_assert_eq!(window.len(), self.size);
        for (index, &reversed) in self.bit_reversal.iter().enumerate() {
            self.re[index] = f64::from(samples[reversed]) * window[reversed];
            self.im[index] = 0.0;
        }

        let mut width = 2;
        while width <= self.size {
            let half = width / 2;
            let twiddle_step = self.size / width;
            for start in (0..self.size).step_by(width) {
                for offset in 0..half {
                    let even = start + offset;
                    let odd = even + half;
                    let twiddle = offset * twiddle_step;
                    let odd_re = self.re[odd] * self.twiddle_re[twiddle]
                        - self.im[odd] * self.twiddle_im[twiddle];
                    let odd_im = self.re[odd] * self.twiddle_im[twiddle]
                        + self.im[odd] * self.twiddle_re[twiddle];
                    let even_re = self.re[even];
                    let even_im = self.im[even];
                    self.re[even] = even_re + odd_re;
                    self.im[even] = even_im + odd_im;
                    self.re[odd] = even_re - odd_re;
                    self.im[odd] = even_im - odd_im;
                }
            }
            width *= 2;
        }
    }
}
