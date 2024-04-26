use std::{ops::Range, path::Path};

pub trait Reader {
    fn get_ranges<'life0, 'life1>(
        &mut self,
        location: &'life0 Path,
        ranges: &'life1 [Range<isize>],
    ) -> anyhow::Result<()>;
}

