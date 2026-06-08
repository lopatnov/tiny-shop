//! `SqliteCatalogSearch` — адаптер порта `CatalogSearch` на SQLite/FTS5 (T1a-5).
//!
//! Реализует поиск и фильтрацию по `product_projection` + `product_attr_index` + `product_fts`.
//! `search()` поддерживает: текст (FTS5 MATCH), фильтры по атрибутам, цене, категории;
//! сортировку и пагинацию. Фасетные счётчики возвращаются пустыми (T2+: Tantivy или агрегат).
//!
//! `upsert()` и `remove()` обновляют те же таблицы напрямую (используется тестами и внешним
//! кодом; event-driven путь — `CatalogProjection::dispatch()`).

use db::ContextDb;
use sqlx::{QueryBuilder, Row, Sqlite};

use crate::{
    CatalogSearch, FilterCond, ProductCard, ProductDoc, SearchError, SearchQuery, SearchResult,
    Sort,
};

pub struct SqliteCatalogSearch {
    db: ContextDb,
}

impl SqliteCatalogSearch {
    pub fn new(db: ContextDb) -> Self {
        Self { db }
    }
}

fn be(e: sqlx::Error) -> SearchError {
    SearchError::Backend(e.to_string())
}

impl CatalogSearch for SqliteCatalogSearch {
    async fn search(&self, query: &SearchQuery) -> Result<SearchResult, SearchError> {
        let limit = query.page.limit.clamp(1, shared::Pagination::MAX_LIMIT) as i64;
        let offset = query.page.offset as i64;

        // FTS text search — returns matching product_ids (empty = no text filter).
        let fts_ids: Option<Vec<String>> = if let Some(ref raw) = query.text {
            let term = build_fts_query(raw);
            if term.is_empty() {
                None
            } else {
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT product_id FROM product_fts WHERE product_fts MATCH ?",
                )
                .bind(&term)
                .fetch_all(&self.db.reader)
                .await
                .map_err(be)?;
                if ids.is_empty() {
                    return Ok(SearchResult {
                        items: vec![],
                        total: 0,
                        facets: vec![],
                    });
                }
                Some(ids)
            }
        } else {
            None
        };

        let total = count_results(&self.db, query, &fts_ids).await?;
        if total == 0 {
            return Ok(SearchResult {
                items: vec![],
                total: 0,
                facets: vec![],
            });
        }

        let order = match query.sort {
            Sort::PriceAsc => "pp.price_minor ASC, pp.id ASC",
            Sort::PriceDesc => "pp.price_minor DESC, pp.id ASC",
            Sort::Newest => "pp.created_at DESC, pp.id ASC",
            Sort::Relevance => "pp.created_at DESC, pp.id ASC",
        };

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT pp.id, pp.title, pp.slug, pp.price_minor, pp.currency, pp.thumb \
             FROM product_projection pp",
        );
        push_where(&mut qb, query, &fts_ids);
        qb.push(format!(" ORDER BY {order} LIMIT "));
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        let rows = qb.build().fetch_all(&self.db.reader).await.map_err(be)?;
        let items = rows
            .into_iter()
            .map(|r| ProductCard {
                product_id: r.get("id"),
                title: r.get("title"),
                slug: r.get("slug"),
                price_minor: r.get("price_minor"),
                currency: r.get("currency"),
                thumb: r.get("thumb"),
            })
            .collect();

        Ok(SearchResult {
            items,
            total,
            facets: vec![], // T1a-5: фасетные счётчики — T2+
        })
    }

    async fn upsert(&self, doc: &ProductDoc) -> Result<(), SearchError> {
        let mut tx = self.db.writer.begin().await.map_err(be)?;

        sqlx::query(
            "INSERT INTO product_projection \
             (id, seller_id, title, slug, description, price_minor, currency, status, category_id, created_at, updated_at) \
             VALUES (?, '', ?, ?, ?, ?, ?, 'published', ?, 0, 0) \
             ON CONFLICT(id) DO UPDATE SET \
               title       = excluded.title, \
               slug        = excluded.slug, \
               description = excluded.description, \
               price_minor = excluded.price_minor, \
               currency    = excluded.currency, \
               category_id = CASE WHEN excluded.category_id != '' \
                                  THEN excluded.category_id \
                                  ELSE product_projection.category_id END",
        )
        .bind(&doc.product_id)
        .bind(&doc.title)
        .bind(&doc.slug)
        .bind(&doc.description)
        .bind(doc.price_minor)
        .bind(&doc.currency)
        .bind(&doc.category_id)
        .execute(&mut *tx)
        .await
        .map_err(be)?;

        // Rebuild FTS from the stored row (attrs from doc.attrs_text, not from product_attr_index).
        delete_fts_tx(&mut tx, &doc.product_id).await.map_err(be)?;
        sqlx::query(
            "INSERT INTO product_fts(product_id, title, description, attrs) VALUES (?, ?, ?, ?)",
        )
        .bind(&doc.product_id)
        .bind(&doc.title)
        .bind(&doc.description)
        .bind(&doc.attrs_text)
        .execute(&mut *tx)
        .await
        .map_err(be)?;

        tx.commit().await.map_err(be)?;
        Ok(())
    }

    async fn remove(&self, product_id: &str) -> Result<(), SearchError> {
        let mut tx = self.db.writer.begin().await.map_err(be)?;

        sqlx::query("DELETE FROM product_attr_index WHERE product_id = ?")
            .bind(product_id)
            .execute(&mut *tx)
            .await
            .map_err(be)?;

        delete_fts_tx(&mut tx, product_id).await.map_err(be)?;

        sqlx::query("DELETE FROM product_projection WHERE id = ?")
            .bind(product_id)
            .execute(&mut *tx)
            .await
            .map_err(be)?;

        tx.commit().await.map_err(be)?;
        Ok(())
    }
}

// -----------------------------------------------------------------
// Построение WHERE-условий (shared between count and select queries)
// -----------------------------------------------------------------

/// Добавляет WHERE-условия в QueryBuilder на основе SearchQuery и (опц.) FTS-результатов.
fn push_where(qb: &mut QueryBuilder<Sqlite>, query: &SearchQuery, fts_ids: &Option<Vec<String>>) {
    qb.push(" WHERE pp.status = 'published'");

    if let Some(ids) = fts_ids {
        qb.push(" AND pp.id IN (");
        let mut sep = qb.separated(", ");
        for id in ids {
            sep.push_bind(id.clone());
        }
        qb.push(")");
    }

    if let Some(ref cat_id) = query.category_id {
        qb.push(
            " AND pp.id IN \
             (SELECT DISTINCT product_id FROM product_attr_index WHERE category_id = ",
        );
        qb.push_bind(cat_id.clone());
        qb.push(")");
    }

    for f in &query.filters {
        push_filter_cond(qb, f);
    }
}

fn push_filter_cond(qb: &mut QueryBuilder<Sqlite>, f: &FilterCond) {
    match f {
        FilterCond::RangePrice {
            min_minor,
            max_minor,
        } => {
            if let Some(min) = min_minor {
                qb.push(" AND pp.price_minor >= ");
                qb.push_bind(*min);
            }
            if let Some(max) = max_minor {
                qb.push(" AND pp.price_minor <= ");
                qb.push_bind(*max);
            }
        }
        FilterCond::CheckboxOr {
            attribute_id,
            values,
        } if !values.is_empty() => {
            qb.push(
                " AND pp.id IN (SELECT product_id FROM product_attr_index \
                 WHERE attribute_id = ",
            );
            qb.push_bind(attribute_id.clone());
            qb.push(" AND val_text IN (");
            let mut sep = qb.separated(", ");
            for v in values {
                sep.push_bind(v.clone());
            }
            qb.push("))");
        }
        FilterCond::EnumAnd {
            attribute_id,
            values,
        } if !values.is_empty() => {
            let n = values.len() as i64;
            qb.push(
                " AND pp.id IN (SELECT product_id FROM product_attr_index \
                 WHERE attribute_id = ",
            );
            qb.push_bind(attribute_id.clone());
            qb.push(" AND val_text IN (");
            let mut sep = qb.separated(", ");
            for v in values {
                sep.push_bind(v.clone());
            }
            qb.push(") GROUP BY product_id HAVING COUNT(DISTINCT val_text) = ");
            qb.push_bind(n);
            qb.push(")");
        }
        FilterCond::Number {
            attribute_id,
            value,
        } => {
            qb.push(
                " AND pp.id IN (SELECT product_id FROM product_attr_index \
                 WHERE attribute_id = ",
            );
            qb.push_bind(attribute_id.clone());
            qb.push(" AND val_num = ");
            qb.push_bind(*value);
            qb.push(")");
        }
        FilterCond::RangeGeneric {
            attribute_id,
            min,
            max,
        } => {
            qb.push(
                " AND pp.id IN (SELECT product_id FROM product_attr_index \
                 WHERE attribute_id = ",
            );
            qb.push_bind(attribute_id.clone());
            if let Some(mn) = min {
                qb.push(" AND val_num >= ");
                qb.push_bind(*mn);
            }
            if let Some(mx) = max {
                qb.push(" AND val_num <= ");
                qb.push_bind(*mx);
            }
            qb.push(")");
        }
        _ => {}
    }
}

async fn count_results(
    db: &ContextDb,
    query: &SearchQuery,
    fts_ids: &Option<Vec<String>>,
) -> Result<u64, SearchError> {
    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("SELECT COUNT(*) FROM product_projection pp");
    push_where(&mut qb, query, fts_ids);
    let count: i64 = qb
        .build_query_scalar()
        .fetch_one(&db.reader)
        .await
        .map_err(be)?;
    Ok(count as u64)
}

// -----------------------------------------------------------------
// FTS helpers
// -----------------------------------------------------------------

async fn delete_fts_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    product_id: &str,
) -> Result<(), sqlx::Error> {
    let rowid: Option<i64> =
        sqlx::query_scalar("SELECT rowid FROM product_fts WHERE product_id = ? LIMIT 1")
            .bind(product_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(rid) = rowid {
        sqlx::query("DELETE FROM product_fts WHERE rowid = ?")
            .bind(rid)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Подготавливает FTS5-запрос: разбивает по словам, добавляет `*` (prefix match).
/// Удаляет символы, специальные для FTS5 (`"`, `(`, `)`, `*`, `-`, `+`), чтобы
/// пользовательский ввод не нарушал синтаксис запроса.
fn build_fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .map(|word| {
            let clean: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("{clean}*")
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
