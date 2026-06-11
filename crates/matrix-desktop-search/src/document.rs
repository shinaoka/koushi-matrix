use std::collections::BTreeMap;

#[derive(Default)]
pub struct SearchDocumentStore {
    documents: BTreeMap<String, ()>,
}

impl SearchDocumentStore {
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }
}

pub struct SearchableEvent;
pub struct SearchCandidate;
pub struct SearchEdit;
