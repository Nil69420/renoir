use super::{LeanCursor, LeanRing};

impl LeanRing {
    pub fn cursor_latest(&self) -> LeanCursor {
        LeanCursor::at(self.published() + 1)
    }

    pub fn cursor_oldest(&self) -> LeanCursor {
        let w = self.published();
        LeanCursor::at(w.saturating_sub(self.mask).max(1))
    }

    pub fn pending(&self, cursor: &LeanCursor) -> usize {
        let w = self.published();
        if cursor.next > w {
            0
        } else {
            (w - cursor.next + 1).min(self.mask + 1) as usize
        }
    }

    pub fn skip_to_latest(&self, cursor: &mut LeanCursor, keep: u64) -> u64 {
        let w = self.published();
        let target = w.saturating_sub(keep.max(1) - 1).max(1);
        if target > cursor.next {
            let skipped = target - cursor.next;
            cursor.lost += skipped;
            cursor.next = target;
            skipped
        } else {
            0
        }
    }

    #[cold]
    pub(super) fn resync(&self, cursor: &mut LeanCursor) {
        let w = self.published();
        let keep = (self.mask + 1) * 3 / 4;
        let target = w.saturating_sub(keep.max(1) - 1).max(1);
        if target > cursor.next {
            cursor.lost += target - cursor.next;
            cursor.next = target;
        }
    }
}
