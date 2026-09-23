use crate::Canvas;

const REV_LEN: usize = 16;
const ID_LEN: usize = 16;

pub fn rev(canvas: &Canvas) -> String {
    blake3::hash(&canvas.canonical_bytes()).to_hex()[..REV_LEN].to_string()
}

// infinite loop to find a fresh id
pub fn fresh_id(canvas: &Canvas, seed: &[u8]) -> String {
    (0u64..)
        .map(|counter| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(seed);
            hasher.update(&counter.to_le_bytes());
            hasher.finalize().to_hex()[..ID_LEN].to_string()
        })
        .find(|id| !is_taken(canvas, id))
        .expect("a 64-bit counter space is never exhausted")
}

// compare [canvas] nodes|edges ids == [id]
fn is_taken(canvas: &Canvas, id: &str) -> bool {
    canvas.nodes.iter().any(|n| n.id == id) || canvas.edges.iter().any(|e| e.id == id)
}
