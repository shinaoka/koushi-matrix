use matrix_desktop_search::SearchDocumentStore;

#[test]
fn search_document_store_can_be_created() {
    let store = SearchDocumentStore::default();

    assert_eq!(store.document_count(), 0);
}
