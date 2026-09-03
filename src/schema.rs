/// Database schema introspection and OWL generation.
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub foreign_keys: Vec<ForeignKey>,
}

pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
}

pub struct ForeignKey {
    pub column: String,
    pub parent_table: String,
    pub parent_column: String,
}

pub struct SchemaIntrospector;

impl SchemaIntrospector {
    /// Convert SQL type name to XSD datatype.
    ///
    /// Recognises common Postgres and DuckDB type names. Parameterised types
    /// like `DECIMAL(18,2)` or `VARCHAR(255)` are normalised by stripping the
    /// `(...)` suffix before matching.
    pub fn sql_to_xsd(sql_type: &str) -> &'static str {
        let lower = sql_type.to_lowercase();
        // Strip parameters: "decimal(18,2)" → "decimal", "varchar(255)" → "varchar".
        let base = match lower.find('(') {
            Some(idx) => lower[..idx].trim().to_string(),
            None => lower.trim().to_string(),
        };
        match base.as_str() {
            "integer" | "int" | "bigint" | "smallint" | "tinyint" | "hugeint"
            | "int4" | "int8" | "int2" | "int1" | "serial" | "bigserial"
            | "smallserial" | "ubigint" | "uinteger" | "usmallint" | "utinyint" => "xsd:integer",
            "numeric" | "decimal" => "xsd:decimal",
            // IEEE 754 binary floats are deliberately NOT xsd:decimal. That
            // type's value space is integers over powers of ten, so it cannot
            // represent NaN, INF or -INF, and asserting it claims an exactness
            // the source column does not have.
            "real" | "float4" => "xsd:float",
            // Bare `float` is dialect-dependent: Postgres reads it as float8,
            // DuckDB as float4 (an alias of REAL). Widen rather than narrow.
            // Every xsd:float value is exactly representable as an xsd:double,
            // so calling a DuckDB single a double overstates the range without
            // misrepresenting any value; the reverse would declare a range
            // narrower than the data. Parameterised `float(p)` normalises to
            // this arm too, which is the same safe direction under Postgres,
            // where p <= 24 would otherwise select real.
            "double precision" | "double" | "float" | "float8" => "xsd:double",
            "boolean" | "bool" => "xsd:boolean",
            "date" => "xsd:date",
            "timestamp" | "timestamptz" | "timestamp without time zone"
            | "timestamp with time zone" | "datetime" => "xsd:dateTime",
            "time" | "time without time zone" | "time with time zone" => "xsd:time",
            "bytea" | "blob" => "xsd:hexBinary",
            "uuid" => "xsd:string",
            _ => "xsd:string",
        }
    }

    /// Convert snake_case table name to PascalCase class name.
    pub fn table_to_class(name: &str) -> String {
        name.split('_')
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().to_string() + &chars.collect::<String>(),
                }
            })
            .collect()
    }

    /// Generate OWL Turtle from introspected schema.
    pub fn generate_turtle(tables: &[TableInfo], base_iri: &str) -> String {
        let mut ttl = String::new();
        ttl.push_str("@prefix owl: <http://www.w3.org/2002/07/owl#> .\n");
        ttl.push_str("@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n");
        ttl.push_str("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n");
        ttl.push_str(&format!("@prefix db: <{}> .\n\n", base_iri));

        // Build FK lookup: (table, column) → parent_table
        let fk_map: std::collections::HashMap<(String, String), &ForeignKey> = tables.iter()
            .flat_map(|t| t.foreign_keys.iter().map(move |fk| ((t.name.clone(), fk.column.clone()), fk)))
            .collect();

        for table in tables {
            let class = Self::table_to_class(&table.name);
            // The IRI-position identifier must be a valid Turtle PN_LOCAL;
            // the human-readable form (pre-sanitization, escaped for safe
            // literal embedding) is kept as the rdfs:label so nothing about
            // the original name is lost (F-025 tranche-1 item 5 -- a table
            // or column name containing a space, parenthesis, or double
            // quote otherwise produced an invalid Turtle document and
            // aborted the whole import; sanitize/escape at emission
            // instead of failing closed on real-world identifiers).
            let class_id = crate::mapping::sanitize_iri(&class);
            let class_label = crate::mapping::escape_ntriples(&class);
            ttl.push_str(&format!("db:{} a owl:Class ;\n    rdfs:label \"{}\" .\n\n", class_id, class_label));

            for col in &table.columns {
                let prop_name = format!("{}_{}", table.name, col.name);
                let prop_id = crate::mapping::sanitize_iri(&prop_name);
                let col_label = crate::mapping::escape_ntriples(&col.name);

                if let Some(fk) = fk_map.get(&(table.name.clone(), col.name.clone())) {
                    // Foreign key → ObjectProperty
                    let parent_class = Self::table_to_class(&fk.parent_table);
                    let parent_class_id = crate::mapping::sanitize_iri(&parent_class);
                    ttl.push_str(&format!("db:{} a owl:ObjectProperty ;\n", prop_id));
                    ttl.push_str(&format!("    rdfs:domain db:{} ;\n", class_id));
                    ttl.push_str(&format!("    rdfs:range db:{} ;\n", parent_class_id));
                    ttl.push_str(&format!("    rdfs:label \"{}\" .\n\n", col_label));
                } else {
                    // Regular column → DatatypeProperty
                    let xsd = Self::sql_to_xsd(&col.data_type);
                    if col.is_primary_key {
                        ttl.push_str(&format!("db:{} a owl:DatatypeProperty , owl:FunctionalProperty ;\n", prop_id));
                    } else {
                        ttl.push_str(&format!("db:{} a owl:DatatypeProperty ;\n", prop_id));
                    }
                    ttl.push_str(&format!("    rdfs:domain db:{} ;\n", class_id));
                    ttl.push_str(&format!("    rdfs:range {} ;\n", xsd));
                    ttl.push_str(&format!("    rdfs:label \"{}\" .\n\n", col_label));
                }

                // NOT NULL → cardinality restriction
                if !col.is_nullable {
                    ttl.push_str(&format!("db:{} rdfs:subClassOf [\n", class_id));
                    ttl.push_str("    a owl:Restriction ;\n");
                    ttl.push_str(&format!("    owl:onProperty db:{} ;\n", prop_id));
                    ttl.push_str("    owl:minCardinality 1\n");
                    ttl.push_str("] .\n\n");
                }
            }
        }

        ttl
    }

    /// Connect to postgres, introspect schema, return TableInfo vec.
    #[cfg(feature = "postgres")]
    pub async fn introspect_postgres(connection_string: &str) -> anyhow::Result<Vec<TableInfo>> {
        use sqlx::postgres::PgPoolOptions;
        use sqlx::Row;

        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(connection_string)
            .await?;

        // Get tables
        let table_rows = sqlx::query(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE'"
        ).fetch_all(&pool).await?;

        let mut tables = Vec::new();

        for trow in &table_rows {
            let table_name: String = trow.get("table_name");

            // Get columns
            let col_rows = sqlx::query(
                "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 ORDER BY ordinal_position"
            ).bind(&table_name).fetch_all(&pool).await?;

            // Get primary keys
            let pk_rows = sqlx::query(
                "SELECT kcu.column_name FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_name = $1"
            ).bind(&table_name).fetch_all(&pool).await?;

            let pk_cols: Vec<String> = pk_rows.iter().map(|r| r.get("column_name")).collect();

            let columns: Vec<ColumnInfo> = col_rows.iter().map(|r| {
                let name: String = r.get("column_name");
                let data_type: String = r.get("data_type");
                let nullable: String = r.get("is_nullable");
                ColumnInfo {
                    is_primary_key: pk_cols.contains(&name),
                    name,
                    data_type,
                    is_nullable: nullable == "YES",
                }
            }).collect();

            // Get foreign keys
            let fk_rows = sqlx::query(
                "SELECT kcu.column_name AS child_column, ccu.table_name AS parent_table, ccu.column_name AS parent_column FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema JOIN information_schema.constraint_column_usage ccu ON tc.constraint_name = ccu.constraint_name WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_name = $1"
            ).bind(&table_name).fetch_all(&pool).await?;

            let foreign_keys: Vec<ForeignKey> = fk_rows.iter().map(|r| ForeignKey {
                column: r.get("child_column"),
                parent_table: r.get("parent_table"),
                parent_column: r.get("parent_column"),
            }).collect();

            tables.push(TableInfo { name: table_name, columns, foreign_keys });
        }

        pool.close().await;
        Ok(tables)
    }

    /// Connect to a DuckDB database (file or in-memory), introspect schema,
    /// return TableInfo vec. Reads from the SQL-standard
    /// `information_schema.tables` / `information_schema.columns` and the
    /// DuckDB-specific `duckdb_constraints()` table function for primary and
    /// foreign keys.
    #[cfg(feature = "duckdb")]
    pub fn introspect_duckdb(target: &str) -> anyhow::Result<Vec<TableInfo>> {
        use duckdb::Connection;

        let conn = if target == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(target)?
        };

        // User tables across ALL user schemas — not just `main`. Frameworks
        // that keep their local catalog in a DuckDB file put tables in their
        // own schema (fenic writes to `typedef_default`). Excluded: SQL system
        // schemas, engine-internal schemas (`__`-prefixed, e.g. fenic's
        // `__fenic_system`), and fenic's telemetry schema `fenic_system`.
        let mut stmt = conn.prepare(
            "SELECT table_schema, table_name FROM information_schema.tables \
             WHERE table_type = 'BASE TABLE' \
               AND table_schema NOT IN ('information_schema', 'pg_catalog', 'fenic_system') \
               AND table_schema NOT LIKE '\\_\\_%' ESCAPE '\\' \
             ORDER BY table_schema, table_name",
        )?;
        let schema_tables: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut tables: Vec<TableInfo> = Vec::new();
        for (schema_name, table_name) in &schema_tables {
            // Columns
            let mut col_stmt = conn.prepare(
                "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
                 WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
            )?;
            let col_rows: Vec<(String, String, String)> = col_stmt
                .query_map([schema_name, table_name], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(col_stmt);

            // Primary keys via duckdb_constraints(). The constraint_column_names
            // column is a list of column names. Convert it to a comma-separated
            // string in SQL because the duckdb-rs crate does not implement
            // `FromSql` for `Vec<String>`.
            let mut pk_stmt = conn.prepare(
                "SELECT array_to_string(constraint_column_names, ',') AS cols \
                 FROM duckdb_constraints() \
                 WHERE schema_name = ? AND table_name = ? AND constraint_type = 'PRIMARY KEY'",
            )?;
            let pk_strings: Vec<String> = pk_stmt
                .query_map([schema_name, table_name], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            drop(pk_stmt);
            let pk_cols: Vec<String> = pk_strings
                .into_iter()
                .flat_map(|s| s.split(',').map(|p| p.trim().to_string()).collect::<Vec<_>>())
                .filter(|s| !s.is_empty())
                .collect();

            // Foreign keys via duckdb_constraints(). Same trick: project list
            // columns to comma-separated strings.
            let mut fk_stmt = conn.prepare(
                "SELECT array_to_string(constraint_column_names, ',') AS child_cols, \
                        referenced_table, \
                        array_to_string(referenced_column_names, ',') AS parent_cols \
                 FROM duckdb_constraints() \
                 WHERE schema_name = ? AND table_name = ? AND constraint_type = 'FOREIGN KEY'",
            )?;
            let fk_rows: Vec<(String, String, String)> = fk_stmt
                .query_map([schema_name, table_name], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(fk_stmt);

            let mut foreign_keys = Vec::new();
            for (child_cols, parent_table, parent_cols) in fk_rows {
                let children: Vec<&str> =
                    child_cols.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                let parents: Vec<&str> =
                    parent_cols.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                for (i, child) in children.iter().enumerate() {
                    if let Some(parent) = parents.get(i) {
                        foreign_keys.push(ForeignKey {
                            column: (*child).to_string(),
                            parent_table: parent_table.clone(),
                            parent_column: (*parent).to_string(),
                        });
                    }
                }
            }

            let columns: Vec<ColumnInfo> = col_rows
                .into_iter()
                .map(|(name, data_type, nullable)| ColumnInfo {
                    is_primary_key: pk_cols.contains(&name),
                    name,
                    data_type,
                    is_nullable: nullable.eq_ignore_ascii_case("YES"),
                })
                .collect();

            // Bare table names feed `table_to_class`; qualify only on a
            // cross-schema collision so single-schema catalogs are unchanged.
            let name = if tables.iter().any(|t: &TableInfo| t.name == *table_name) {
                format!("{}_{}", schema_name, table_name)
            } else {
                table_name.clone()
            };
            tables.push(TableInfo {
                name,
                columns,
                foreign_keys,
            });
        }

        Ok(tables)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::GraphStore;

    // ─── F-025 tranche-1 item 5 / §5b: `import-schema` emits invalid ──────
    // ─── Turtle for identifiers with spaces/parens ─────────────────────────
    //
    // Any column or table name containing a space or parenthesis (ubiquitous
    // in real databases and anything imported from spreadsheets) was
    // interpolated raw into a Turtle prefixed name (`db:{name}`), whose
    // local part cannot contain them -- the generated document then failed
    // the tool's own validation, and because `main.rs` builds ONE Turtle
    // document for all tables and validates it before loading anything, one
    // bad column aborted the whole import.

    fn table_with_bad_column() -> TableInfo {
        TableInfo {
            name: "orders".to_string(),
            columns: vec![
                ColumnInfo {
                    name: "id".to_string(),
                    data_type: "integer".to_string(),
                    is_nullable: false,
                    is_primary_key: true,
                },
                ColumnInfo {
                    name: "Latency (ms)".to_string(),
                    data_type: "integer".to_string(),
                    is_nullable: true,
                    is_primary_key: false,
                },
            ],
            foreign_keys: vec![],
        }
    }

    #[test]
    fn generated_turtle_with_a_parenthesised_column_name_parses_cleanly() {
        let tables = vec![table_with_bad_column()];
        let ttl = SchemaIntrospector::generate_turtle(&tables, "http://example.org/data/");

        GraphStore::validate_turtle(&ttl).unwrap_or_else(|e| {
            panic!("generate_turtle produced invalid Turtle for a column name \
                    containing a space and parentheses: {e}\n---\n{ttl}")
        });
    }

    #[test]
    fn the_original_column_name_survives_as_a_label() {
        let tables = vec![table_with_bad_column()];
        let ttl = SchemaIntrospector::generate_turtle(&tables, "http://example.org/data/");
        assert!(
            ttl.contains(r#"rdfs:label "Latency (ms)""#),
            "the original (readable) column name must be kept as an \
             rdfs:label, sanitizing only the IRI-position identifier:\n{ttl}"
        );
    }

    #[test]
    fn a_table_name_with_a_space_also_parses_cleanly() {
        let tables = vec![TableInfo {
            name: "Order Items".to_string(),
            columns: vec![ColumnInfo {
                name: "id".to_string(),
                data_type: "integer".to_string(),
                is_nullable: false,
                is_primary_key: true,
            }],
            foreign_keys: vec![],
        }];
        let ttl = SchemaIntrospector::generate_turtle(&tables, "http://example.org/data/");
        GraphStore::validate_turtle(&ttl).unwrap_or_else(|e| {
            panic!("generate_turtle produced invalid Turtle for a table name \
                    containing a space: {e}\n---\n{ttl}")
        });
    }

    #[test]
    fn a_double_quote_in_a_name_does_not_break_the_label_literal() {
        let tables = vec![TableInfo {
            name: "widgets".to_string(),
            columns: vec![ColumnInfo {
                name: r#"size ("inches")"#.to_string(),
                data_type: "integer".to_string(),
                is_nullable: true,
                is_primary_key: false,
            }],
            foreign_keys: vec![],
        }];
        let ttl = SchemaIntrospector::generate_turtle(&tables, "http://example.org/data/");
        GraphStore::validate_turtle(&ttl).unwrap_or_else(|e| {
            panic!("generate_turtle produced invalid Turtle for a column name \
                    containing a double-quote: {e}\n---\n{ttl}")
        });
    }
}
