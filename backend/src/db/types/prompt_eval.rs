#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PromptEval {
    pub id: i64,
    pub prompt_id: i64,
    pub system_prompt_input: Option<String>,
    pub user_prompt_input: String,
    pub name: String,
    pub evaluation_type: String,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}
