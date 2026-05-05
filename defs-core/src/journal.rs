use alloc::vec::Vec;
pub type BlockNum = u64;

pub enum JournalOp {
    WriteBlock { block: BlockNum, data: Vec<u8> },
    WriteInode { inode: u64, data: Vec<u8> },
    AllocBlock { block: BlockNum },
    FreeBlock { block: BlockNum },
    AllocInode { inode: u64 },
    FreeInode { inode: u64 },
    UpdateSuperblock { data: Vec<u8> },
}

pub struct JournalEntry {
    pub seq: u64,
    pub transaction_id: u64,
    pub ops: Vec<JournalOp>,
    pub committed: bool,
}

pub struct Journal {
    pub entries: Vec<JournalEntry>,
    pub start_block: BlockNum,
    pub size_blocks: u32,
    pub next_seq: u64,
    pub next_txn: u64,
    pub head: u64,
    pub tail: u64,
}

impl Journal {
    pub fn new(start: BlockNum, size: u32) -> Self {
        Self {
            entries: Vec::new(),
            start_block: start,
            size_blocks: size,
            next_seq: 1,
            next_txn: 1,
            head: 0,
            tail: 0,
        }
    }

    pub fn begin_transaction(&mut self) -> u64 {
        let txn = self.next_txn;
        self.next_txn += 1;
        self.entries.push(JournalEntry {
            seq: self.next_seq,
            transaction_id: txn,
            ops: Vec::new(),
            committed: false,
        });
        self.next_seq += 1;
        txn
    }

    pub fn add_op(&mut self, txn_id: u64, op: JournalOp) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.transaction_id == txn_id) {
            entry.ops.push(op);
        }
    }

    pub fn commit(&mut self, txn_id: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.transaction_id == txn_id) {
            entry.committed = true;
        }
    }

    pub fn abort(&mut self, txn_id: u64) {
        self.entries.retain(|e| e.transaction_id != txn_id);
    }

    pub fn recover(&self) -> Vec<&JournalEntry> {
        self.entries.iter().filter(|e| e.committed).collect()
    }

    pub fn checkpoint(&mut self) {
        self.entries.retain(|e| !e.committed);
    }

    pub fn pending_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.committed).count()
    }
}
