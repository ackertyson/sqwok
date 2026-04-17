use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub uuid: Uuid,
    pub screenname: String,
}
