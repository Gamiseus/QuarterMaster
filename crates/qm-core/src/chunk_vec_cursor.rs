#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use itertools::Itertools;

// Extension trait
pub trait IntoCursor {
    type Cursor;
    fn into_cursor(self) -> Self::Cursor;
}

/// A regex cursor over a rope's chunks. An empty rope has no chunk: the cursor then shows
/// one empty slice.
pub struct ChunkVecCursor<'a> {
    chunks: Vec<&'a str>,
    idx: usize,
    len: usize,
    offset: usize,
}

impl<'a> regex_cursor::Cursor for ChunkVecCursor<'a> {
    fn chunk(&self) -> &[u8] {
        self.chunks.get(self.idx).map_or(&[], |c| c.as_bytes())
    }

    fn advance(&mut self) -> bool {
        let next = self.idx.saturating_add(1);
        if next >= self.chunks.len() {
            return false;
        }
        let Some(cur) = self.chunks.get(self.idx) else {
            return false;
        };
        self.offset = self.offset.saturating_add(cur.len());
        self.idx = next;
        true
    }

    fn backtrack(&mut self) -> bool {
        let Some(prev_idx) = self.idx.checked_sub(1) else {
            return false;
        };
        let Some(prev) = self.chunks.get(prev_idx) else {
            return false;
        };
        self.idx = prev_idx;
        self.offset = self.offset.saturating_sub(prev.len());
        true
    }

    fn total_bytes(&self) -> Option<usize> {
        Some(self.len)
    }

    fn offset(&self) -> usize {
        self.offset
    }
}

impl<'a> IntoCursor for &'a crop::Rope {
    type Cursor = ChunkVecCursor<'a>;

    fn into_cursor(self) -> Self::Cursor {
        ChunkVecCursor {
            chunks: self.chunks().collect_vec(),
            idx: 0,
            len: self.byte_len(),
            offset: 0,
        }
    }
}

impl<'a> IntoCursor for crop::RopeSlice<'a> {
    type Cursor = ChunkVecCursor<'a>;

    fn into_cursor(self) -> Self::Cursor {
        ChunkVecCursor {
            chunks: self.chunks().collect_vec(),
            idx: 0,
            len: self.byte_len(),
            offset: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex_cursor::Cursor;

    #[test]
    fn empty_rope_has_an_empty_chunk() {
        let rope = crop::Rope::new();
        let mut c = (&rope).into_cursor();
        assert!(c.chunk().is_empty());
        assert!(!c.advance());
        assert!(!c.backtrack());
        assert_eq!(c.offset(), 0);
        assert_eq!(c.total_bytes(), Some(0));
    }
}
