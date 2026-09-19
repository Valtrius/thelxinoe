//! Chromaprint algorithm 1: 1365 sample hop at 11025 Hz, with 19 filter hops.
//! Match only diverse, sustained audio; do not infer recap/preview from repetition.
use std::collections::{HashMap, HashSet};
pub const STEP: f64 = 1365.0 / 11025.0;
const DELAY: f64 = (19.0 * 1365.0 + 2731.0) / 11025.0;
pub fn decode(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_le_bytes(*b))
        .collect()
}
pub fn recurring(a: &[u32], b: &[u32]) -> Option<(f64, f64, f64, f64)> {
    if a.len() < 160 || b.len() < 160 {
        return None;
    }
    let mut index = HashMap::<u16, Vec<usize>>::new();
    for (j, hash) in b.iter().enumerate() {
        let values = index.entry((hash & 0xffff) as u16).or_default();
        if values.len() < 32 {
            values.push(j);
        }
    }
    let mut votes = HashMap::<isize, usize>::new();
    for (i, hash) in a.iter().enumerate().step_by(3) {
        if let Some(hits) = index.get(&((hash & 0xffff) as u16)) {
            for j in hits {
                if (hash ^ b[*j]).count_ones() <= 4 {
                    *votes.entry(*j as isize - i as isize).or_default() += 1;
                }
            }
        }
    }
    let mut shifts = votes.into_iter().collect::<Vec<_>>();
    shifts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let mut best = None;
    let mut longest = 0;
    for (shift, _) in shifts.into_iter().take(12) {
        let start = 0.max(-shift) as usize;
        let end = a.len().min((b.len() as isize - shift).max(0) as usize);
        let mut run = start;
        let mut last_good = start;
        let mut errors = 0;
        for (i, hash) in a.iter().enumerate().take(end).skip(start) {
            let j = (i as isize + shift) as usize;
            if (hash ^ b[j]).count_ones() <= 6 {
                last_good = i;
                errors = 0;
            } else {
                errors += 1;
            }
            if errors > 5 || i + 1 == end {
                let length = last_good.saturating_sub(run);
                if length > longest
                    && length >= 160
                    && a[run..=last_good]
                        .iter()
                        .copied()
                        .collect::<HashSet<_>>()
                        .len()
                        > 30
                {
                    longest = length;
                    best = Some((
                        run as f64 * STEP + DELAY,
                        last_good as f64 * STEP,
                        (run as isize + shift) as f64 * STEP + DELAY,
                        (last_good as isize + shift) as f64 * STEP,
                    ));
                }
                run = i + 1;
                last_good = run;
                errors = 0;
            }
        }
    }
    best.filter(|(start, end, _, _)| end - start >= 15.0 && end - start <= 300.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_shifted_audio_but_rejects_silence_and_short_motifs() {
        let sequence = (0..250)
            .map(|n| (n as u32).wrapping_mul(2654435761))
            .collect::<Vec<_>>();
        let mut a = vec![0; 90];
        a.extend(&sequence);
        a.extend(vec![0; 100]);
        let mut b = vec![u32::MAX; 140];
        b.extend(sequence.iter().map(|h| h ^ 0x10000));
        b.extend(vec![u32::MAX; 100]);
        let (start, end, other, other_end) = recurring(&a, &b).unwrap();
        assert!((other - start - 50.0 * STEP).abs() < 0.01);
        assert!((end - other_end + 50.0 * STEP).abs() < 0.01);
        assert!(start > 10.0 && end < 45.0);
        assert!(recurring(&vec![0; 400], &vec![0; 400]).is_none());
        assert!(recurring(&sequence[..100], &sequence[..100]).is_none());
    }
}
