//! xoshiro256++, small and fast. Written here instead of taken from a crate
//! so a seed replays the same game forever, whatever crate versions do.

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // splitmix64 to spread the seed over the state
        let mut x = seed;
        let mut next = || {
            x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        Rng { s: [next(), next(), next(), next()] }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let out = (s[0].wrapping_add(s[3])).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }

    /// Uniform in 0..n. The bias is under 2^-32, fine for a game.
    pub fn below(&mut self, n: u32) -> u32 {
        debug_assert!(n > 0);
        (((self.next_u64() >> 32) * n as u64) >> 32) as u32
    }

    pub fn coin(&mut self) -> bool {
        self.next_u64() >> 63 == 1
    }

    pub fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            xs.swap(i, j);
        }
    }
}
