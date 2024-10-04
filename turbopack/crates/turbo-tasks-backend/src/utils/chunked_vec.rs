pub struct ChunkedVec<T> {
    chunks: Vec<Vec<T>>,
}

impl<T> ChunkedVec<T> {
    pub fn new() -> Self {
        Self { chunks: Vec::new() }
    }

    pub fn push(&mut self, item: T) {
        if let Some(chunk) = self.chunks.last_mut() {
            if chunk.len() < chunk.capacity() {
                chunk.push(item);
                return;
            }
        }
        let mut chunk = Vec::with_capacity(chunk_size(self.chunks.len()));
        chunk.push(item);
        self.chunks.push(chunk);
    }
}

fn chunk_size(chunk_index: usize) -> usize {
    8 << chunk_index
}
