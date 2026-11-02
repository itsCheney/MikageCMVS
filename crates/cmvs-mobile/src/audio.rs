//! Sound output for hosts that pull it from a real-time audio callback.
//!
//! The engine's channels play into a rodio mixer. Pulling that mixer decodes
//! Vorbis and takes locks, which a real-time callback must not do, so a feeder
//! thread of our own mixes ahead into a lock-free ring and the callback only
//! copies out of it. While the host's audio is stopped the ring stays full and
//! the feeder waits, so the sounds pause with it.
use rodio::mixer::{Mixer, MixerSource};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 2;
/// Frames mixed per pass of the feeder.
const CHUNK: usize = 256;
/// Frames held ahead of the callback: about 93 ms at 44.1 kHz. Sound positions
/// the game reads run this far ahead of what is heard.
const AHEAD: usize = 4096;

/// A mixer for the engine and its source for [`AudioOutput`].
pub fn mixer() -> (Mixer, MixerSource) {
    rodio::mixer::mixer(
        std::num::NonZero::new(CHANNELS).expect("non-zero"),
        std::num::NonZero::new(SAMPLE_RATE).expect("non-zero"),
    )
}

/// The feeder thread and the ring's reading end.
pub struct AudioOutput {
    ring: rtrb::Consumer<f32>,
    stop: Arc<AtomicBool>,
    feeder: Option<JoinHandle<()>>,
}

impl AudioOutput {
    pub fn new(mut source: MixerSource) -> std::io::Result<Self> {
        let (mut producer, ring) = rtrb::RingBuffer::<f32>::new(usize::from(CHANNELS) * AHEAD);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let feeder = std::thread::Builder::new()
            .name("cmvs-audio".into())
            .spawn(move || {
                let mut buf = vec![0.0f32; usize::from(CHANNELS) * CHUNK];
                while !stopping.load(Ordering::Relaxed) {
                    if producer.slots() < buf.len() {
                        std::thread::sleep(Duration::from_millis(3));
                        continue;
                    }
                    // An empty mixer yields None; keep counting samples so the
                    // channels stay interleaved in step.
                    for sample in &mut buf {
                        *sample = source.next().unwrap_or(0.0);
                    }
                    if let Ok(chunk) = producer.write_chunk_uninit(buf.len()) {
                        chunk.fill_from_iter(buf.iter().copied());
                    }
                }
            })?;
        Ok(Self {
            ring,
            stop,
            feeder: Some(feeder),
        })
    }

    /// Fills `out` (interleaved stereo) from the ring; what the feeder has not
    /// mixed yet is silence. Never blocks or allocates.
    pub fn render(&mut self, out: &mut [f32]) {
        let n = self.ring.slots().min(out.len()) & !1;
        let mut done = 0;
        if let Ok(chunk) = self.ring.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            out[..a.len()].copy_from_slice(a);
            out[a.len()..a.len() + b.len()].copy_from_slice(b);
            done = a.len() + b.len();
            chunk.commit_all();
        }
        out[done..].fill(0.0);
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.feeder.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    #[test]
    fn plays_a_source_added_to_the_mixer_and_silence_otherwise() {
        let (mixer, source) = mixer();
        let mut out = AudioOutput::new(source).unwrap();
        let mut buf = vec![1.0f32; 512];
        std::thread::sleep(Duration::from_millis(20));
        out.render(&mut buf);
        assert!(buf.iter().all(|&s| s == 0.0));
        mixer.add(
            rodio::source::SineWave::new(440.0)
                .amplify(0.5)
                .take_duration(Duration::from_secs(1)),
        );
        let mut heard = false;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(5));
            out.render(&mut buf);
            if buf.iter().any(|&s| s.abs() > 0.1) {
                heard = true;
                break;
            }
        }
        assert!(heard);
    }
}
