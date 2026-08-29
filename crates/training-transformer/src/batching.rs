use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPlan {
    batches: Vec<Vec<usize>>,
}

impl BatchPlan {
    pub fn deterministic(member_ids: &[Uuid], batch_size: usize, seed: u64, epoch: u32) -> Self {
        assert!(batch_size > 0, "validated batch size");
        let mut order = (0..member_ids.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| {
            stable_score(seed ^ u64::from(epoch), member_ids[*index].as_bytes())
        });
        Self {
            batches: order.chunks(batch_size).map(<[usize]>::to_vec).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.batches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    pub fn batch(&self, index: usize) -> Option<&[usize]> {
        self.batches.get(index).map(Vec::as_slice)
    }
}

fn stable_score(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::BatchPlan;

    fn member_ids() -> Vec<Uuid> {
        (1..=7).map(Uuid::from_u128).collect()
    }

    #[test]
    fn plans_bounded_batches_deterministically_by_seed_and_epoch() {
        let member_ids = member_ids();
        let first = BatchPlan::deterministic(&member_ids, 3, 42, 1);
        let repeated = BatchPlan::deterministic(&member_ids, 3, 42, 1);
        let next_epoch = BatchPlan::deterministic(&member_ids, 3, 42, 2);

        assert_eq!(first, repeated);
        assert_ne!(first, next_epoch);
        assert_eq!(first.len(), 3);
        assert!(first.batches.iter().all(|batch| batch.len() <= 3));
        let mut members = first.batches.into_iter().flatten().collect::<Vec<_>>();
        members.sort_unstable();
        assert_eq!(members, (0..7).collect::<Vec<_>>());
    }
}
