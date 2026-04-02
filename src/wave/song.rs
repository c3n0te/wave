#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Song {
    song_id: i64,
    title: String,
    artist: String,
}

#[allow(dead_code)]
impl Song {
    pub fn new(song_id: i64, title: String, artist: String) -> Song {
        Self {
            song_id: song_id,
            title: title,
            artist: artist,
        }
    }

    pub fn song_id(&self) -> i64 {
        self.song_id
    }

    pub fn title(&self) -> String {
        self.title.clone()
    }

    pub fn artist(&self) -> String {
        self.artist.clone()
    }
}
