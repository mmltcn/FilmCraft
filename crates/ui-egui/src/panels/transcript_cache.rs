//! Derived transcript data survives playhead movement and UI redraws, but never project edits.

use std::ops::Range;
use std::sync::{Arc, Weak};

use filmcraft_edit::transcript::{SeqWord, paragraphs, search};
use filmcraft_engine::Session;
use filmcraft_project::{ItemId, Project};
use filmcraft_time::Tick;

#[derive(Default)]
pub(crate) struct Content {
    pub words: Vec<SeqWord>,
    pub paragraphs: Vec<Range<usize>>,
}

#[derive(Default)]
pub(crate) struct TranscriptCache {
    // A weak key neither retains a closed project nor allows an allocation address to be reused.
    project: Weak<Project>,
    view: (bool, Option<ItemId>),
    content: Arc<Content>,
    query: Option<String>,
    highlights: Arc<Vec<bool>>,
}

impl TranscriptCache {
    pub fn content(&mut self, session: &Session, source: bool) -> Arc<Content> {
        let view = (source, if source { session.state.source_item } else { session.state.active_sequence });
        let project = Arc::downgrade(&session.project);
        if !self.project.ptr_eq(&project) || self.view != view {
            let words = if source { filmcraft_engine::transcript::source_words(session) } else { filmcraft_engine::transcript::sequence_words(session) };
            let paragraphs = paragraphs(&words, Tick::from_seconds_f64(1.5));
            self.content = Arc::new(Content { words, paragraphs });
            self.project = project;
            self.view = view;
            self.query = None;
        }
        self.content.clone()
    }

    pub fn highlights(&mut self, query: &str) -> Arc<Vec<bool>> {
        if self.query.as_deref() != Some(query) {
            let mut flags = vec![false; self.content.words.len()];
            // Search ranges can overlap. Mark their union in linear time rather than checking
            // every match for every word on every redraw.
            let mut marked = 0;
            for hit in search(&self.content.words, query) {
                let start = hit.start.max(marked);
                if let Some(slice) = flags.get_mut(start..hit.end) {
                    slice.fill(true);
                }
                marked = marked.max(hit.end);
            }
            self.highlights = Arc::new(flags);
            self.query = Some(query.to_owned());
        }
        self.highlights.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cached_source_refreshes_after_correction_undo_and_project_replacement() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let item = s.active_sequence().unwrap().audio_tracks[0].items[0].item;
        s.execute("source.open", json!({"item": item.0})).unwrap();
        s.execute(
            "transcript.set",
            json!({"item": item.0, "transcript": {"words": [
            {"text": "hello", "start": 0, "end": 100}, {"text": "hello", "start": 100, "end": 200},
            {"text": "hello", "start": 200, "end": 300}]}}),
        )
        .unwrap();
        let mut cache = TranscriptCache::default();
        let first = cache.content(&s, true);
        assert_eq!(&*cache.highlights("hello hello"), &[true, true, true]);
        assert_eq!(&*cache.highlights("missing"), &[false, false, false]);
        s.state.source_playhead = Tick(150);
        assert!(Arc::ptr_eq(&first, &cache.content(&s, true)));
        s.execute("transcript.correctWord", json!({"item": item.0, "index": 1, "text": "world"})).unwrap();
        assert_eq!(cache.content(&s, true).words[1].text, "world");
        assert_eq!(&*cache.highlights("hello hello"), &[false, false, false]);
        s.undo();
        assert_eq!(cache.content(&s, true).words[1].text, "hello");
        s.execute("file.openDemoProject", json!({})).unwrap();
        assert!(cache.content(&s, true).words.is_empty());
        assert!(cache.highlights("hello").is_empty());
    }

    #[test]
    fn switching_between_source_and_sequence_does_not_reuse_the_other_view() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let item = s.active_sequence().unwrap().audio_tracks[0].items[0].item;
        s.execute("source.open", json!({"item": item.0})).unwrap();
        s.execute(
            "transcript.set",
            json!({"item": item.0, "transcript": {"words": [
            {"text": "hello", "start": 0, "end": 100}]}}),
        )
        .unwrap();
        let mut cache = TranscriptCache::default();
        let source = cache.content(&s, true);
        assert_eq!(source.words, filmcraft_engine::transcript::source_words(&s));
        let sequence = cache.content(&s, false);
        assert!(!Arc::ptr_eq(&source, &sequence));
        assert_eq!(sequence.words, filmcraft_engine::transcript::sequence_words(&s));
        s.state.source_item = None;
        assert!(cache.content(&s, true).words.is_empty());
    }
}
