//! Merkle tree over verdict record lines. Leaves and inner nodes use different
//! one-byte prefixes, and an unpaired node is promoted (never duplicated), so
//! two different record lists can never share a root.
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Side {
    #[serde(rename = "L")]
    Left,
    #[serde(rename = "R")]
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub side: Side,
    pub hash: [u8; 32],
}

pub fn leaf(line: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([0x00]);
    h.update(line);
    h.finalize().into()
}

pub fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([0x01]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

fn next_level(level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    level
        .chunks(2)
        .map(|pair| if pair.len() == 2 { node(&pair[0], &pair[1]) } else { pair[0] })
        .collect()
}

pub fn root(leaves: &[[u8; 32]]) -> [u8; 32] {
    assert!(!leaves.is_empty(), "merkle root of an empty batch");
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        level = next_level(&level);
    }
    level[0]
}

pub fn proof(leaves: &[[u8; 32]], mut index: usize) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut level = leaves.to_vec();
    while level.len() > 1 {
        let sibling = if index.is_multiple_of(2) { index + 1 } else { index - 1 };
        if sibling < level.len() {
            let side = if sibling < index { Side::Left } else { Side::Right };
            steps.push(Step { side, hash: level[sibling] });
        }
        level = next_level(&level);
        index /= 2;
    }
    steps
}

/// The side of every step in the proof for leaf `index` of a `count`-leaf tree. A proof does
/// not bind its leaf's position by itself (an unpaired node is promoted without a step), so a
/// verifier compares the proof's sides with these to tie the proof to one position.
pub fn expected_sides(count: usize, mut index: usize) -> Vec<Side> {
    let (mut sides, mut len) = (Vec::new(), count);
    while len > 1 {
        if index.is_multiple_of(2) {
            if index + 1 < len {
                sides.push(Side::Right);
            }
        } else {
            sides.push(Side::Left);
        }
        index /= 2;
        len = len.div_ceil(2);
    }
    sides
}

pub fn verify(leaf: [u8; 32], proof: &[Step], root: [u8; 32]) -> bool {
    let acc = proof.iter().fold(leaf, |acc, step| match step.side {
        Side::Left => node(&step.hash, &acc),
        Side::Right => node(&acc, &step.hash),
    });
    acc == root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<[u8; 32]> {
        (0..n).map(|i| leaf(format!("{{\"line\":{i}}}").as_bytes())).collect()
    }

    #[test]
    fn leaf_and_node_are_domain_separated() {
        let a = leaf(b"x");
        // A leaf over the bytes of an inner node's preimage must not equal that node.
        let mut pre = Vec::new();
        pre.extend_from_slice(&a);
        pre.extend_from_slice(&a);
        assert_ne!(leaf(&pre), node(&a, &a));
    }

    #[test]
    fn single_leaf_is_its_own_root() {
        let l = leaves(1);
        assert_eq!(root(&l), l[0]);
        assert!(proof(&l, 0).is_empty());
        assert!(verify(l[0], &[], l[0]));
    }

    #[test]
    fn three_leaves_promote_the_unpaired_node() {
        let l = leaves(3);
        let expected = node(&node(&l[0], &l[1]), &l[2]);
        assert_eq!(root(&l), expected);
    }

    #[test]
    fn every_leaf_proves_and_only_its_own_position() {
        for n in [1usize, 2, 3, 5, 7, 8, 9, 256] {
            let l = leaves(n);
            let r = root(&l);
            for i in 0..n {
                let p = proof(&l, i);
                assert!(verify(l[i], &p, r), "n={n} i={i}");
                if n > 1 {
                    let j = (i + 1) % n;
                    assert!(!verify(l[j], &p, r), "proof of {i} must not verify {j} (n={n})");
                }
            }
        }
    }

    #[test]
    fn expected_sides_match_every_generated_proof() {
        for n in 1usize..=40 {
            let l = leaves(n);
            for i in 0..n {
                let sides: Vec<Side> = proof(&l, i).iter().map(|s| s.side).collect();
                assert_eq!(expected_sides(n, i), sides, "n={n} i={i}");
            }
        }
        assert!(expected_sides(1, 0).is_empty());
        assert_eq!(expected_sides(3, 2), [Side::Left]);
        assert_eq!(expected_sides(3, 0), [Side::Right, Side::Right]);
        assert_eq!(expected_sides(5, 4), [Side::Left]);
    }

    #[test]
    fn expected_sides_differ_between_positions() {
        for n in [2usize, 3, 5, 7, 8, 9] {
            for i in 0..n {
                for j in 0..n {
                    if i != j {
                        assert_ne!(expected_sides(n, i), expected_sides(n, j), "n={n} {i} vs {j}");
                    }
                }
            }
        }
    }

    #[test]
    fn tampering_breaks_verification() {
        let l = leaves(5);
        let r = root(&l);
        let mut p = proof(&l, 2);
        assert!(!verify(leaf(b"tampered"), &p, r));
        p[0].side = match p[0].side { Side::Left => Side::Right, Side::Right => Side::Left };
        assert!(!verify(l[2], &p, r));
    }
}
