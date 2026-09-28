//! Per-instrument effects after the voices: a stereo chorus for width and a
//! ping-pong echo. Saturation lives in the voice, before the filter.

use std::f32::consts::TAU;

use crate::dsp::{DelayLine, OnePole};
use crate::presets::Fx;

pub struct Effects {
    sample_rate: f32,
    chorus_mix: f32,
    chorus: [DelayLine; 2],
    chorus_phase: f32,
    delay_mix: f32,
    delay_feedback: f32,
    delay_samples: f32,
    echo: [DelayLine; 2],
    /// Each repeat a little darker, as tape and analog delays do.
    damp: [OnePole; 2],
}

impl Effects {
    pub fn new(fx: &Fx, sample_rate: f32) -> Self {
        let chorus_len = if fx.chorus > 0.0 { (0.03 * sample_rate) as usize } else { 4 };
        let echo_len = if fx.delay_mix > 0.0 { (fx.delay_time * sample_rate) as usize + 8 } else { 4 };
        Self {
            sample_rate,
            chorus_mix: fx.chorus,
            chorus: [DelayLine::new(chorus_len), DelayLine::new(chorus_len)],
            chorus_phase: 0.0,
            delay_mix: fx.delay_mix,
            delay_feedback: fx.delay_feedback.clamp(0.0, 0.85),
            delay_samples: fx.delay_time * sample_rate,
            echo: [DelayLine::new(echo_len), DelayLine::new(echo_len)],
            damp: [OnePole::new(4_500.0, sample_rate), OnePole::new(4_500.0, sample_rate)],
        }
    }

    pub fn clear(&mut self) {
        for d in self.chorus.iter_mut().chain(self.echo.iter_mut()) {
            d.clear();
        }
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.chorus_mix > 0.0 {
            let rate = 0.35 / self.sample_rate;
            let base = 0.012 * self.sample_rate;
            let depth = 0.0035 * self.sample_rate;
            let wet = self.chorus_mix;
            let dry = 1.0 - wet * 0.5;
            for i in 0..left.len() {
                let (l, r) = (left[i], right[i]);
                self.chorus[0].push(l);
                self.chorus[1].push(r);
                // The two sides move in quadrature, which is what spreads them.
                let a = (self.chorus_phase * TAU).sin();
                let b = (self.chorus_phase * TAU).cos();
                let wl = self.chorus[0].read(base + depth * a);
                let wr = self.chorus[1].read(base + depth * b);
                left[i] = l * dry + wl * wet;
                right[i] = r * dry + wr * wet;
                self.chorus_phase = (self.chorus_phase + rate).fract();
            }
        }
        if self.delay_mix > 0.0 {
            let (fb, mix, d) = (self.delay_feedback, self.delay_mix, self.delay_samples);
            for i in 0..left.len() {
                let tap_l = self.echo[0].read(d);
                let tap_r = self.echo[1].read(d);
                let mono = (left[i] + right[i]) * 0.5;
                // Ping-pong: the dry signal enters on the left, and each
                // repeat crosses to the other side.
                let back_l = self.damp[0].process(tap_r * fb);
                let back_r = self.damp[1].process(tap_l * fb);
                self.echo[0].push(mono + back_l);
                self.echo[1].push(back_r);
                left[i] += tap_l * mix;
                right[i] += tap_r * mix;
            }
        }
    }
}
