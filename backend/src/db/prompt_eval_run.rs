use anyhow::Result;

use super::types::prompt_eval_run::{PromptEvalRun, PromptEvalVersionPerformance};

#[derive(Clone, Debug)]
pub struct PromptEvalTestRunRepository {
    pool: sqlx::SqlitePool,
}

impl PromptEvalTestRunRepository {
    pub async fn new(pool: sqlx::SqlitePool) -> Result<Self> {
        Ok(PromptEvalTestRunRepository { pool })
    }

    pub async fn get_by_id(&self, id: i64) -> Result<PromptEvalRun> {
        sqlx::query_as!(
            PromptEvalRun,
            r#"
            SELECT
                per.*,
                pe.name AS prompt_eval_name
            FROM prompt_eval_run per
            JOIN prompt_eval pe ON per.prompt_eval_id = pe.id
            WHERE per.id = ?
            "#,
            id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_by_prompt_version(
        &self,
        prompt_version_id: i64,
    ) -> Result<Vec<PromptEvalRun>> {
        sqlx::query_as!(
            PromptEvalRun,
            r#"
            SELECT
                per.*,
                pe.name AS prompt_eval_name
            FROM prompt_eval_run per
            JOIN prompt_eval pe ON per.prompt_eval_id = pe.id
            WHERE per.prompt_version_id = ?
            "#,
            prompt_version_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_prompt_version_performance(
        &self,
        prompt_id: i64,
    ) -> Result<Vec<PromptEvalVersionPerformance>> {
        let query = r#"
            WITH avg_scores AS (
                SELECT 
                    pv.id AS version_id,
                    pv.version_number,
                    pv.created_at AS version_date,
                    CAST(ROUND(AVG(CAST(per.score AS FLOAT)), 2) AS FLOAT) AS avg_score,
                    COUNT(per.id) AS run_count
                FROM prompt_version pv
                INNER JOIN prompt_eval_run per
                    ON pv.id = per.prompt_version_id
                WHERE per.score IS NOT NULL
                  AND pv.prompt_id = ?
                GROUP BY pv.id, pv.version_number, pv.created_at
            )
            SELECT 
                version_id,
                version_number,
                version_date,
                COALESCE(avg_score, 0.0) AS avg_score,
                run_count
            FROM avg_scores
            ORDER BY version_number ASC
        "#;

        let rows = sqlx::query_as::<_, PromptEvalVersionPerformance>(query)
            .bind(prompt_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }


    pub async fn create(
        &self,
        run_id: &str,
        prompt_version_id: i64,
        prompt_eval_id: i64,
        score: Option<i64>,
        output: &str,
    ) -> Result<PromptEvalRun> {
        let result = sqlx::query!(
            r#"
            INSERT INTO prompt_eval_run (run_id, prompt_version_id, prompt_eval_id, score, output)
            VALUES (?, ?, ?, ?, ?)
            "#,
            run_id,
            prompt_version_id,
            prompt_eval_id,
            score,
            output
        )
        .execute(&self.pool)
        .await?;

        let id = result.last_insert_rowid();

        self.get_by_id(id).await
    }

    pub async fn update_score(
        &self,
        id: i64,
        score: i64,
    ) -> Result<PromptEvalRun> {
        sqlx::query!(
            r#"
            UPDATE prompt_eval_run
            SET SCORE = ?
            WHERE ID = ?
            "#,
            score,
            id,
        )
        .execute(&self.pool)
        .await?;

        self.get_by_id(id).await
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        sqlx::query!(
            r#"
            DELETE FROM prompt_eval_run
            WHERE id = ?
            "#,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
