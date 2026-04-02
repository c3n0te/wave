#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Fingerprint {
    song_id: i64,
    hash: u32,
    anchor_time: f64,
}

#[allow(dead_code)]
impl Fingerprint {
    pub fn new(song_id: i64, hash: u32, anchor_time: f64) -> Fingerprint {
        Self {
            song_id,
            hash,
            anchor_time,
        }
    }

    pub fn song_id(&self) -> i64 {
        self.song_id
    }

    pub fn hash(&self) -> u32 {
        self.hash
    }

    pub fn anchor_time(&self) -> f64 {
        self.anchor_time
    }
}
