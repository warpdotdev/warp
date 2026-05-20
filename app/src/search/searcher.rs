#![cfg(not(target_family = "wasm"))] // Tantivy is not supported for wasm target as of now.

use anyhow::Context;
use futures::FutureExt as _;
use instant::Instant;
use itertools::Itertools;
use parking_lot::{Mutex, RwLock};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::iter::Peekable;
use std::sync::Arc;
use std::thread::available_parallelism;
use std::time::Duration;
use string_offset::ByteOffset;
use strum_macros::Display;
use tantivy::collector::TopDocs;
use tantivy::query::{
    AllQuery, BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, PhrasePrefixQuery, TermQuery,
};
use tantivy::schema::{
    BytesOptions, Field, FieldEntry, IndexRecordOption, OwnedValue, Schema, TextFieldIndexing,
    STORED, TEXT,
};
use tantivy::tokenizer::{Token, TokenStream, Tokenizer};
use tantivy::{
    snippet::SnippetGenerator, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
};
use warpui::r#async::{block_on, executor::Background, Timer};

pub type FullTextSearchDocumentEntry = HashMap<String, FullTextSearchFieldValue>;

/// Maximum number of threads per index writer.
pub const MAX_THREADS_PER_INDEX_WRITER: usize = 2;
/// Minimum memory budget for the index is 15MB
pub const MIN_MEMORY_BUDGET: usize = 15_000_000;
/// Default memory budget for the index is 50MB
pub const DEFAULT_MEMORY_BUDGET: usize = 50_000_000;
/// Used to normalize search scores to a ranges of roughly 0-5. Complex queries might have a score exceeding 5.
const SCORE_BOOST_FACTOR: f32 = 5.0;
/// Used to penalize fuzzy matches in the score, so full-text term searches will take priority.
const FUZZY_SCORE_PENALIZED_FACTOR: f32 = 0.8;
/// Used to convert the (boosted and normalized) Tantivy score to a similar magnitude as the old fuzzy search score.
pub const SCORE_CONVERSION_FACTOR: f64 = 50.0; // TODO: get rid of this if we're not blending results.
/// Default max number of results to return from a search query
const DEFAULT_MAX_RESULT_COUNT: usize = 20;
/// Field name for the composite key that uniquely identifies documents
const COMPOSITE_KEY_FIELD: &str = "_composite_key";

#[derive(Debug, Clone, Copy, Display)]
#[allow(unused)]
pub enum FullTextSearchFieldTypes {
    Str,
    U64,
    I64,
    F64,
    Bool,
}

impl FullTextSearchFieldTypes {
    fn field_entry_from_name(&self, name: String) -> FieldEntry {
        match self {
            FullTextSearchFieldTypes::Str => FieldEntry::new_text(name, STORED.into()),
            FullTextSearchFieldTypes::U64 => FieldEntry::new_u64(name, STORED.into()),
            FullTextSearchFieldTypes::I64 => FieldEntry::new_i64(name, STORED.into()),
            FullTextSearchFieldTypes::F64 => FieldEntry::new_f64(name, STORED.into()),
            FullTextSearchFieldTypes::Bool => FieldEntry::new_bool(name, STORED.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FullTextSearchFieldValue {
    Str(String),
    U64(u64),
    I64(i64),
    F64(f64),
    Bool(bool),
}

impl From<String> for FullTextSearchFieldValue {
    fn from(value: String) -> Self {
        FullTextSearchFieldValue::Str(value)
    }
}

impl From<u64> for FullTextSearchFieldValue {
    fn from(value: u64) -> Self {
        FullTextSearchFieldValue::U64(value)
    }
}

impl From<i64> for FullTextSearchFieldValue {
    fn from(value: i64) -> Self {
        FullTextSearchFieldValue::I64(value)
    }
}

impl From<f64> for FullTextSearchFieldValue {
    fn from(value: f64) -> Self {
        FullTextSearchFieldValue::F64(value)
    }
}

impl From<bool> for FullTextSearchFieldValue {
    fn from(value: bool) -> Self {
        FullTextSearchFieldValue::Bool(value)
    }
}

impl From<usize> for FullTextSearchFieldValue {
    fn from(value: usize) -> Self {
        FullTextSearchFieldValue::U64(value as u64)
    }
}

impl FullTextSearchFieldValue {
    #[allow(clippy::wrong_self_convention)]
    pub fn to_owned_value(self) -> OwnedValue {
        match self {
            FullTextSearchFieldValue::Str(value) => OwnedValue::Str(value),
            FullTextSearchFieldValue::U64(value) => OwnedValue::U64(value),
            FullTextSearchFieldValue::I64(value) => OwnedValue::I64(value),
            FullTextSearchFieldValue::F64(value) => OwnedValue::F64(value),
            FullTextSearchFieldValue::Bool(value) => OwnedValue::Bool(value),
        }
    }

    pub fn from_owned_value(value: OwnedValue) -> Option<FullTextSearchFieldValue> {
        match value {
            OwnedValue::Str(s) => Some(FullTextSearchFieldValue::Str(s)),
            OwnedValue::U64(n) => Some(FullTextSearchFieldValue::U64(n)),
            OwnedValue::I64(n) => Some(FullTextSearchFieldValue::I64(n)),
            OwnedValue::F64(n) => Some(FullTextSearchFieldValue::F64(n)),
            OwnedValue::Bool(b) => Some(FullTextSearchFieldValue::Bool(b)),
            _ => None,
        }
    }
}

/// Represents a single search match result.
#[derive(Debug, Clone)]
struct FullTextSearchMatchUntyped {
    /// The relevance score from tantivy (higher is more relevant). Range is 0-5 inclusive.
    pub score: f64,
    /// The values of the fields of the matched result, by field name.
    pub values: HashMap<String, OwnedValue>,
    /// Character positions in the description text that should be highlighted, by search field name
    pub highlights: HashMap<String, Vec<usize>>,
}

#[derive(Debug, Clone)]
pub struct FullTextSearchMatch<V: FullTextSearchMatchValues, H: FullTextSearchMatchHighlights> {
    /// The relevance score from tantivy (higher is more relevant). Range is 0-5 inclusive.
    pub score: f64,
    /// The values of the fields of the matched result, by field name.
    pub values: V,
    /// Character positions in the description text that should be highlighted, by search field name.
    pub highlights: H,
}

pub trait FullTextSearchMatchValues {
    fn from_match_result_values(values: HashMap<String, OwnedValue>) -> Option<Self>
    where
        Self: Sized;
}

pub trait FullTextSearchMatchHighlights {
    fn from_match_result_highlights(values: HashMap<String, Vec<usize>>) -> Option<Self>
    where
        Self: Sized;
}

struct SearcherReaderWrapper {
    search_index: Arc<Index>,
    weighted_search_fields: HashMap<String, (Field, f32)>,
    id_fields: HashMap<String, (Field, FullTextSearchFieldTypes)>,
    normalizing_factor: f32,
    reader: Option<IndexReader>,
}

/// A thread-safe handle with internal mutability to the reader that allows
/// lazy construction of the reader.
type SearcherReaderHandle = Arc<RwLock<SearcherReaderWrapper>>;

/// This wrapper contains the minimum required information to perform read (search) operations.
impl SearcherReaderWrapper {
    fn new(
        search_index: Arc<Index>,
        weighted_search_fields: HashMap<String, (Field, f32)>,
        id_fields: HashMap<String, (Field, FullTextSearchFieldTypes)>,
        normalizing_factor: f32,
    ) -> Self {
        // The writer will manually reload the reader after each write operation to ensure it is up-to-date.
        let reader = search_index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .ok();
        Self {
            search_index,
            weighted_search_fields,
            id_fields,
            normalizing_factor,
            reader,
        }
    }

    fn get_all_documents(
        &self,
        include_search_fields: bool,
    ) -> anyhow::Result<Vec<HashMap<String, OwnedValue>>> {
        let Some(reader) = &self.reader else {
            // Assume the reader is built on the first write operation, if building-on-construction fails.
            // Therefore, if the reader is not set, return an empty vector.
            return Ok(vec![]);
        };
        let searcher = reader.searcher();

        // Create an AllQuery to match all documents
        let all_query = AllQuery {};

        // Search for all documents
        let all_docs = searcher
            .search(
                &all_query,
                &TopDocs::with_limit(searcher.num_docs().max(1) as usize).order_by_score(),
            )
            .with_context(|| "Failed to execute search for all documents")?;

        Ok(all_docs
            .into_iter()
            .filter_map(|(_, doc_address)| {
                // Retrieve the document.
                let retrieved_doc: TantivyDocument = searcher.doc(doc_address).ok()?;
                let mut values = HashMap::new();

                // Extract the id fields.
                for (field_name, (field, _)) in &self.id_fields {
                    let field_value = retrieved_doc.get_first(*field)?.into();
                    values.insert(field_name.clone(), field_value);
                }

                // Extract the search fields if include_sensitive is true
                if include_search_fields {
                    for (field_name, (field, _)) in &self.weighted_search_fields {
                        // Search fields should be strings.
                        let value = retrieved_doc.get_first(*field)?.into();
                        if matches!(value, OwnedValue::Str(_)) {
                            values.insert(field_name.clone(), value);
                        } else {
                            return None;
                        }
                    }
                }

                Some(values)
            })
            .collect())
    }

    fn search(
        &self,
        search_term: &str,
        include_search_fields: bool,
    ) -> anyhow::Result<Vec<FullTextSearchMatchUntyped>> {
        // Split the search term into words.
        let words: Vec<&str> = search_term.split_whitespace().collect();

        // Return empty results if there are no words.
        if words.is_empty() {
            return Ok(vec![]);
        }

        let Some(reader) = &self.reader else {
            // Assume the reader is built on the first write operation, if building-on-construction fails.
            // Therefore, if the reader is not set, return an empty vector.
            return Ok(vec![]);
        };

        let searcher = reader.searcher();

        // Build a collection of subqueries to combine in a boolean query.
        let mut subqueries: Vec<(Occur, Box<dyn tantivy::query::Query>)> = vec![];

        // Add term queries for all words except the last one.
        if words.len() > 1 {
            for word in &words[0..words.len() - 1] {
                for (field, weight) in self.weighted_search_fields.values() {
                    let term = Term::from_field_text(*field, word);
                    let term_query = build_term_query(term);
                    let weighted_query = Box::new(BoostQuery::new(
                        term_query,
                        // Boost the term query by the field weight, normalized by the total weight so the final
                        // score is in the range of roughly 0-5. Complex queries might have a score exceeding 5.
                        *weight * SCORE_BOOST_FACTOR / self.normalizing_factor,
                    ));
                    subqueries.push((Occur::Should, weighted_query));
                }
            }
        }

        // Last word would be an "or" query:
        // - Either a regex (prefix) query
        // - Or a full term query
        let last_word = words[words.len() - 1]; // safe because we checked words is not empty

        for (field, weight) in self.weighted_search_fields.values() {
            let term = Term::from_field_text(*field, last_word);
            let term_query = build_term_query(term.clone());
            let prefix_query = Box::new(PhrasePrefixQuery::new(vec![term]));

            let bool_query = Box::new(BooleanQuery::new(vec![
                (Occur::Should, prefix_query),
                (Occur::Should, term_query),
            ]));
            // The entire "or" query should have the same weighting as a single term query
            let weighted_query = Box::new(BoostQuery::new(
                bool_query,
                // Boost the term query by the field weight, normalized by the total weight so the final
                // score is in the range of roughly 0-5. Complex queries might have a score exceeding 5.
                *weight * SCORE_BOOST_FACTOR / self.normalizing_factor,
            ));
            subqueries.push((Occur::Should, weighted_query));
        }

        // Create the boolean query from collected subqueries.
        let bool_query = BooleanQuery::new(subqueries);

        // Search for top documents.
        let top_docs = searcher
            .search(
                &bool_query,
                &TopDocs::with_limit(DEFAULT_MAX_RESULT_COUNT).order_by_score(),
            )
            .with_context(|| "Failed to execute search")?;

        let search_matches = top_docs
            .into_iter()
            .filter_map(|(score, doc_address)| {
                // Retrieve the document.
                let retrieved_doc: TantivyDocument = searcher.doc(doc_address).ok()?;
                let mut values = HashMap::new();
                let mut highlights = HashMap::new();

                // Extract the id fields.
                for (field_name, (field, _)) in &self.id_fields {
                    let field_value = retrieved_doc.get_first(*field)?;
                    values.insert(field_name.clone(), field_value.into());
                }

                // Generate highlight information.
                for (field_name, (field, _)) in &self.weighted_search_fields {
                    // Search fields should be strings.
                    let OwnedValue::Str(field_value) = retrieved_doc.get_first(*field)?.into()
                    else {
                        return None;
                    };

                    let snippet_generator =
                        SnippetGenerator::create(&searcher, &bool_query, *field).ok()?;
                    let snippet = snippet_generator.snippet_from_doc(&retrieved_doc);

                    // The snippet might not start from the beginning of the field value. The exclusion condition is set to always false
                    // since we always want to highlight the first match.
                    let mut matched_indices = if let Some(snippet_start_offset) =
                        prefix_start_index(&field_value, snippet.fragment(), |_| false)
                    {
                        let start_offset = snippet_start_offset.as_usize();
                        snippet
                            .highlighted()
                            .iter()
                            .flat_map(|range| range.start + start_offset..range.end + start_offset)
                            .collect()
                    } else {
                        HashSet::new()
                    };

                    // TODO: Tantivy does not support snippets for regex queries. As a workaround for now, we'll highlight the prefix manually
                    // See https://github.com/quickwit-oss/tantivy/issues/672
                    if let Some(prefix_start_offset) =
                        prefix_start_index(&field_value, last_word, |idx| {
                            matched_indices.contains(&idx)
                        })
                    {
                        let start_offset = prefix_start_offset.as_usize();
                        matched_indices.extend(start_offset..start_offset + last_word.len());
                    }

                    if include_search_fields {
                        values.insert(field_name.clone(), OwnedValue::Str(field_value));
                    }

                    highlights.insert(
                        field_name.clone(),
                        matched_indices.into_iter().sorted().collect_vec(),
                    );
                }

                // Add a SearchMatch with the binding ID, score and highlighting information
                let search_match = FullTextSearchMatchUntyped {
                    score: score as f64,
                    values,
                    highlights,
                };

                Some(search_match)
            })
            .collect_vec();

        Ok(search_matches)
    }

    /// Try to construct it using `builder_fn` if the reader is not set.
    /// ### Returns
    /// - `Ok(())` if successful.
    /// - `Ok(())` without calling `builder_fn` if the reader is set (no-op).
    /// - The error if `builder_fn` fails.
    fn try_set_reader(
        &mut self,
        builder_fn: impl Fn() -> anyhow::Result<IndexReader>,
    ) -> anyhow::Result<()> {
        if self.reader.is_some() {
            return Ok(());
        }
        self.reader = Some(builder_fn()?);
        Ok(())
    }

    /// Reload the reader if it is set. Returns an error the reload fails, or,
    /// unlike [`Self::try_set_reader`], if the reader is not set.
    fn try_reload(&self) -> anyhow::Result<()> {
        let Some(reader) = &self.reader else {
            anyhow::bail!("Calling reload on an unset reader handle")
        };
        Ok(reader.reload()?)
    }
}

enum WriterOperation {
    Insert(TantivyDocument),
    Delete(Term),
    Clear,
}

/// This wrapper contains the minimum required information to perform write operations.
struct SearcherWriterWrapper {
    search_index: Arc<Index>,
    id_field_names: HashSet<String>,
    /// The field used to store the composite key hash.
    composite_key_field: Field,
    writer: Option<IndexWriter<TantivyDocument>>,
    /// Memory budget for the writer in bytes.
    memory_budget: usize,
    /// Handle to the reader wrapper. This is present here for initialization of the reader on write.
    reader_handle: SearcherReaderHandle,
}

/// A thread-safe handle to the writer wrapper, which can be shared across threads.
type SearcherWriterHandle = Arc<Mutex<SearcherWriterWrapper>>;

impl SearcherWriterWrapper {
    fn new(
        reader_handle: SearcherReaderHandle,
        composite_key_field: Field,
        memory_budget: usize,
    ) -> Self {
        let handle_copy = reader_handle.clone();
        let guard = handle_copy.read();
        let memory_budget = memory_budget.max(MIN_MEMORY_BUDGET);
        let search_index = guard.search_index.clone();
        let num_threads = std::cmp::min(
            available_parallelism().map(|ap| ap.get()).unwrap_or(1),
            MAX_THREADS_PER_INDEX_WRITER,
        );
        let writer = search_index
            .writer_with_num_threads(memory_budget, num_threads)
            .ok();

        SearcherWriterWrapper {
            search_index,
            memory_budget,
            composite_key_field,
            writer,
            reader_handle,
            id_field_names: guard.id_fields.keys().cloned().collect(),
        }
    }

    fn execute_operations(
        &mut self,
        events: impl IntoIterator<Item = SearcherEvent>,
    ) -> anyhow::Result<()> {
        // Initialize the reader if it is not already set (i.e. it failed to initialize on construction).
        self.reader_handle.write().try_set_reader(|| {
            // We will manually reload the reader after each writer operation to ensure it is up-to-date.
            Ok(self
                .search_index
                .reader_builder()
                .reload_policy(ReloadPolicy::Manual)
                .try_into()?)
        })?;

        let mut ops = vec![];
        for event in events.into_iter() {
            match event {
                SearcherEvent::DocumentDeleted(entry) => {
                    // Generate composite key hash from identifying entry
                    let composite_key_hash = self.generate_composite_key_hash(&entry);
                    // Create deletion search term for the composite key field and delete matching documents
                    let term =
                        Term::from_field_bytes(self.composite_key_field, &composite_key_hash);
                    ops.push(WriterOperation::Delete(term));
                }
                SearcherEvent::DocumentInserted(entry) => {
                    // Overwrite the document if it already exists.
                    let composite_key_hash = self.generate_composite_key_hash(&entry);
                    let term =
                        Term::from_field_bytes(self.composite_key_field, &composite_key_hash);
                    ops.push(WriterOperation::Delete(term));
                    ops.push(WriterOperation::Insert(self.entry_to_document(entry)?));
                }
                SearcherEvent::IndexCleared => ops.push(WriterOperation::Clear),
            }
        }

        let writer = self.writer()?;
        for op in ops.into_iter() {
            match op {
                WriterOperation::Insert(document) => {
                    writer.add_document(document)?;
                }
                WriterOperation::Delete(term) => {
                    writer.delete_term(term);
                }
                WriterOperation::Clear => {
                    writer.delete_all_documents()?;
                }
            }
        }

        if let Err(e) = writer.commit() {
            log::error!("Failed to commit index writer: {e}");
            writer.rollback()?;
            anyhow::bail!("Failed to commit bulk modification to index");
        }
        // Manually reload the reader to ensure it is up-to-date.
        self.reader_handle.read().try_reload()?;
        Ok(())
    }

    fn clear(&mut self) -> anyhow::Result<()> {
        self.execute_operations([SearcherEvent::IndexCleared])
    }

    fn insert_document(&mut self, entry: FullTextSearchDocumentEntry) -> anyhow::Result<()> {
        self.execute_operations([SearcherEvent::DocumentInserted(entry)])
    }

    fn delete_document(
        &mut self,
        identifying_entry: FullTextSearchDocumentEntry,
    ) -> anyhow::Result<()> {
        self.execute_operations([SearcherEvent::DocumentDeleted(identifying_entry)])
    }

    fn entry_to_document(
        &self,
        entry: FullTextSearchDocumentEntry,
    ) -> anyhow::Result<TantivyDocument> {
        let mut document = TantivyDocument::default();
        // Generate and add composite key hash
        let composite_key_hash = self.generate_composite_key_hash(&entry);
        document.add_field_value(
            self.composite_key_field,
            &OwnedValue::Bytes(composite_key_hash),
        );

        for (name, value) in entry.into_iter() {
            let field = self.search_index.schema().get_field(&name)?;
            document.add_field_value(field, &value.to_owned_value());
        }
        Ok(document)
    }

    /// Generate a composite key hash from a document entry containing ID fields.
    /// The hash is generated by sorting field names and hashing their values with field names
    /// to ensure uniqueness and efficiency.
    fn generate_composite_key_hash(&self, doc_entry: &FullTextSearchDocumentEntry) -> Vec<u8> {
        let mut id_pairs: Vec<(&String, &FullTextSearchFieldValue)> = doc_entry
            .iter()
            .filter(|(name, _)| self.id_field_names.contains(*name))
            .collect();
        id_pairs.sort_by(|(a, _), (b, _)| a.cmp(b));

        let mut hasher = Sha256::new();
        // Add each field-value pair to the hasher in sorted order
        for (name, value) in id_pairs {
            hasher.update(name.as_bytes());
            hasher.update(b":"); // separator

            match value {
                FullTextSearchFieldValue::Str(s) => hasher.update(s.as_bytes()),
                FullTextSearchFieldValue::U64(n) => hasher.update(n.to_le_bytes()),
                FullTextSearchFieldValue::I64(n) => hasher.update(n.to_le_bytes()),
                FullTextSearchFieldValue::F64(n) => hasher.update(n.to_le_bytes()),
                FullTextSearchFieldValue::Bool(b) => hasher.update([*b as u8]),
            };
            hasher.update(b"|"); // separator between fields
        }

        hasher.finalize().to_vec()
    }

    /// Waits for any remaining merging threads to complete, then drops
    /// the writer.
    ///
    /// This function blocks until the merging threads have completed, so
    /// be cautious when using it from an async context.
    fn wait_on_and_drop_indexing_threads(&mut self) -> anyhow::Result<()> {
        if let Some(writer) = self.writer.take() {
            writer.wait_merging_threads()?;
        }
        Ok(())
    }

    fn writer(&mut self) -> anyhow::Result<&mut IndexWriter<TantivyDocument>> {
        if self.writer.is_none() {
            let writer = self.search_index.writer(self.memory_budget)?;
            self.writer = Some(writer);
        }

        // We can safely unwrap here because we just set it above.
        Ok(self.writer.as_mut().expect("Writer should be initialized"))
    }
}

/// Performs full-text search operations based on a defined schema.
///
/// Manages a Tantivy index in-memory, allowing indexing of documents and
/// searching using a specific strategy:
/// - Term queries for all words except the last.
/// - A prefix regex query (`word.*`) for the last word.
/// - Results are boosted based on field weights.
///
/// If reader/writer initialization attempt fails on construction, the writer will reattempt initialization on the _write_ operations
/// until success. This is to avoid error handling when constructing the searcher.
pub struct SimpleFullTextSearcher<C: SearchSchemaConfig> {
    writer: SearcherWriterHandle,
    reader: SearcherReaderHandle,
    _marker: std::marker::PhantomData<C>,
}

impl<C: SearchSchemaConfig> SimpleFullTextSearcher<C> {
    pub fn new(schema: &FullTextSearchSchema<C>, memory_budget: usize) -> Self {
        let mut schema_builder = Schema::builder();

        // Add composite key field for efficient term querying
        let composite_key_field = schema_builder.add_bytes_field(
            COMPOSITE_KEY_FIELD,
            BytesOptions::default().set_indexed().set_stored(),
        );

        let mut weighted_search_fields = HashMap::new();
        let mut normalizing_factor = 0.0;
        for (field_name, weight) in schema.weighted_search_fields.iter() {
            let text_indexing = TEXT
                .get_indexing_options()
                .cloned()
                .unwrap_or(
                    TextFieldIndexing::default()
                        .set_index_option(IndexRecordOption::WithFreqsAndPositions),
                )
                .set_tokenizer("custom");
            let text_option = TEXT.clone().set_indexing_options(text_indexing) | STORED;

            let field = schema_builder.add_text_field(field_name, text_option);
            weighted_search_fields.insert(field_name.clone(), (field, *weight));
            normalizing_factor += weight;
        }

        let mut id_fields = HashMap::new();
        for (field_name, field_type) in schema.id_fields.iter() {
            let field =
                schema_builder.add_field(field_type.field_entry_from_name(field_name.clone()));
            id_fields.insert(field_name.clone(), (field, *field_type));
        }

        let search_index = Arc::new(Index::create_in_ram(schema_builder.build()));
        search_index
            .tokenizers()
            .register("custom", CustomTokenizer::default());

        let reader = Arc::new(RwLock::new(SearcherReaderWrapper::new(
            search_index,
            weighted_search_fields,
            id_fields,
            // The normalization is done via division, so in order to boost the score, we divide by the boost factor.
            normalizing_factor / schema.boost_factor,
        )));

        let writer = Arc::new(Mutex::new(SearcherWriterWrapper::new(
            reader.clone(),
            composite_key_field,
            memory_budget,
        )));

        SimpleFullTextSearcher {
            writer,
            reader,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn clear_search_index(&mut self) -> anyhow::Result<()> {
        self.writer.lock().clear()
    }

    pub fn build_index(
        &self,
        documents: impl IntoIterator<Item = C::SearchDocEntry>,
    ) -> anyhow::Result<()> {
        self.writer.lock().execute_operations(
            documents
                .into_iter()
                .map(|entry| SearcherEvent::DocumentInserted(entry.into_document_entry())),
        )
    }

    #[allow(unused)]
    /// Inserts a document into the index. It will overwrite any existing document with the same composite key hash.
    pub fn insert_document(&self, entry: C::SearchDocEntry) -> anyhow::Result<()> {
        self.writer
            .lock()
            .insert_document(entry.into_document_entry())
    }

    /// Deletes document(s) that match the composite key hash generated from the identifying_entry.
    ///
    /// This method generates a composite key hash from all fields in the identifying_entry,
    /// then uses a term deletion to efficiently remove matching documents.
    #[allow(unused)]
    pub fn delete_document(&self, identifying_entry: C::SearchIdEntry) -> anyhow::Result<()> {
        self.writer
            .lock()
            .delete_document(identifying_entry.into_identifying_entry())
    }

    #[allow(unused)]
    pub fn get_all_documents(&self) -> Result<Vec<C::SearchDocEntry>, anyhow::Error> {
        Ok(self
            .reader
            .read()
            .get_all_documents(true)?
            .into_iter()
            .filter_map(|doc_values| {
                let result = C::SearchDocEntry::from_match_result_values(doc_values);
                if result.is_none() {
                    log::error!("Failed to convert search result values into structured data");
                }
                result
            })
            .collect())
    }

    #[allow(unused)]
    pub fn get_all_doc_ids(&self) -> Result<Vec<C::SearchIdEntry>, anyhow::Error> {
        Ok(self
            .reader
            .read()
            .get_all_documents(false)?
            .into_iter()
            .filter_map(|doc_values| {
                let result = C::SearchIdEntry::from_match_result_values(doc_values);
                if result.is_none() {
                    log::error!("Failed to convert search result values into structured data");
                }
                result
            })
            .collect())
    }

    pub fn search_id(
        &self,
        search_term: &str,
    ) -> anyhow::Result<Vec<FullTextSearchMatch<C::SearchIdEntry, C::SearchHighlight>>> {
        Ok(self
            .reader
            .read()
            .search(search_term, false)?
            .into_iter()
            .filter_map(|search_match| {
                let Some(values) = C::SearchIdEntry::from_match_result_values(search_match.values)
                else {
                    log::error!("Failed to convert search result values into structured data");
                    return None;
                };
                let Some(highlights) =
                    C::SearchHighlight::from_match_result_highlights(search_match.highlights)
                else {
                    log::error!("Failed to convert search result highlights into structured data");
                    return None;
                };
                Some(FullTextSearchMatch {
                    score: search_match.score,
                    values,
                    highlights,
                })
            })
            .collect())
    }

    pub fn search_full_doc(
        &self,
        search_term: &str,
    ) -> anyhow::Result<Vec<FullTextSearchMatch<C::SearchDocEntry, C::SearchHighlight>>> {
        Ok(self
            .reader
            .read()
            .search(search_term, true)?
            .into_iter()
            .filter_map(|search_match| {
                let Some(values) = C::SearchDocEntry::from_match_result_values(search_match.values)
                else {
                    log::error!("Failed to convert search result values into structured data");
                    return None;
                };
                let Some(highlights) =
                    C::SearchHighlight::from_match_result_highlights(search_match.highlights)
                else {
                    log::error!("Failed to convert search result highlights into structured data");
                    return None;
                };
                Some(FullTextSearchMatch {
                    score: search_match.score,
                    values,
                    highlights,
                })
            })
            .collect())
    }
}

fn build_term_query(term: Term) -> Box<BooleanQuery> {
    let term_query = Box::new(TermQuery::new(
        term.clone(),
        IndexRecordOption::WithFreqsAndPositions,
    ));
    let fuzzy_query = Box::new(FuzzyTermQuery::new(term, 1, true));
    // The fuzzy query should be weaker than the term query.
    let boosted_query = Box::new(BoostQuery::new(fuzzy_query, FUZZY_SCORE_PENALIZED_FACTOR));
    Box::new(BooleanQuery::new(vec![
        (Occur::Should, term_query),
        (Occur::Should, boosted_query),
    ]))
}

/// Tokenize the text by splitting on whitespaces and punctuation.
#[derive(Clone, Default)]
pub struct CustomTokenizer {
    token: Token,
}

/// TokenStream produced by the `CustomTokenizer`.
pub struct CustomTokenStream<'a> {
    tokens_iter: Peekable<<Vec<(usize, &'a str)> as IntoIterator>::IntoIter>,
    token: &'a mut Token,
}

impl Tokenizer for CustomTokenizer {
    type TokenStream<'a> = CustomTokenStream<'a>;
    fn token_stream<'a>(&'a mut self, text: &'a str) -> CustomTokenStream<'a> {
        self.token.reset();
        CustomTokenStream::new(&mut self.token, text)
    }
}

impl<'a> CustomTokenStream<'a> {
    fn new(token: &'a mut Token, text: &'a str) -> Self {
        let mut tokens = vec![];

        // Initial split is based on "hard" delimiters like spaces or punctuations that are not specially permitted.
        split_with_offsets(text, |c| {
            !(Self::char_permitted_in_simple_token(c) || Self::is_simple_token_separator(c))
        })
        .into_iter()
        .for_each(|(outer_offset, composite_token)| {
            tokens.push((outer_offset, composite_token));
            // Second split is based on "soft" delimiters like '-' or '/'.
            let second_split = split_with_offsets(composite_token, Self::is_simple_token_separator);
            if second_split.len() <= 1 {
                return;
            }

            second_split.into_iter().for_each(|(offset, simple_token)| {
                tokens.push((offset + outer_offset, simple_token));
                // The inner split is based on '_'.
                let inner_split = split_with_offsets(simple_token, Self::is_sub_token_separator);
                if inner_split.len() <= 1 {
                    return;
                }

                inner_split
                    .into_iter()
                    .for_each(|(inner_offset, sub_token)| {
                        tokens.push((inner_offset + offset + outer_offset, sub_token));
                    });
            })
        });

        CustomTokenStream {
            tokens_iter: tokens.into_iter().peekable(),
            token,
        }
    }

    fn is_sub_token_separator(c: char) -> bool {
        c == '_'
    }

    fn char_permitted_in_simple_token(c: char) -> bool {
        c.is_alphanumeric() || Self::is_sub_token_separator(c)
    }

    /// Whether the character is a separator for simple tokens (i.e. whether
    /// it is a special character permitted in composite tokens)
    fn is_simple_token_separator(c: char) -> bool {
        ['-', '/', '\\', ':'].contains(&c)
    }
}

impl TokenStream for CustomTokenStream<'_> {
    fn advance(&mut self) -> bool {
        self.token.text.clear();
        self.token.position = self.token.position.wrapping_add(1);
        let Some((offset_from, token)) = self.tokens_iter.next() else {
            return false;
        };

        let offset_to = offset_from + token.len();
        self.token.offset_from = offset_from;
        self.token.offset_to = offset_to;
        self.token.text.push_str(token);
        true
    }

    fn token(&self) -> &Token {
        self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        self.token
    }
}

/// This is a "fast" search function, since the fragments will only be split between tokens.
/// This function searches for a possible starting byte index of a prefix in the text.
fn prefix_start_index(
    text: &str,
    prefix: &str,
    exclusion_condition: impl Fn(usize) -> bool,
) -> Option<ByteOffset> {
    let mut current_word_start_idx = 0;
    for (byte_idx, c) in text.char_indices() {
        if !c.is_alphanumeric() {
            let suffix = &text[current_word_start_idx..];
            // We do not want to count for certain indices - for example, when this function is used to compute
            // the highlight of prefix queries, we want to ignore the indices that are already matched by the term search
            // (i.e. if the term search already highlighted "hello", we don't want to try to match the prefix "hel" of the same word again)
            if suffix.starts_with(prefix) && !exclusion_condition(current_word_start_idx) {
                return Some(current_word_start_idx.into());
            }
            // Start of the next word
            current_word_start_idx = byte_idx + c.len_utf8();
        }
    }
    // If the last character is not a separator, we need to check the last word.
    // For example, if the text is "hello world" and the prefix is "wor", we need to check "world".
    if current_word_start_idx < text.len() {
        let suffix = &text[current_word_start_idx..];
        if suffix.starts_with(prefix) && !exclusion_condition(current_word_start_idx) {
            return Some(current_word_start_idx.into());
        }
    }
    None
}

fn split_with_offsets(s: &str, pred: impl Fn(char) -> bool) -> Vec<(usize, &str)> {
    let mut result = Vec::new();
    let mut current_start = 0;

    for (byte_idx, ch) in s.char_indices() {
        if pred(ch) {
            if current_start < byte_idx {
                let sub_str = &s[current_start..byte_idx];
                if !sub_str.trim().is_empty() {
                    result.push((current_start, &s[current_start..byte_idx]));
                }
            }
            current_start = byte_idx + ch.len_utf8();
        }
    }

    // Handle the last segment if it exists
    if current_start < s.len() {
        result.push((current_start, &s[current_start..]));
    }

    result
}

/// Represents the schema for full-text search, including weighted search fields and ID fields.
/// A schema is required to create a [`SimpleFullTextSearcher`].
///
/// Documents to be indexed in the searcher must be created using the schema same schema as the searcher
/// via [`FullTextSearchSchema::create_document_entry`]
/// to ensure consistency in field types and names.
#[derive(Default, Debug)]
pub struct FullTextSearchSchema<C: SearchSchemaConfig> {
    weighted_search_fields: HashMap<String, f32>,
    id_fields: HashMap<String, FullTextSearchFieldTypes>,
    boost_factor: f32,
    _marker: std::marker::PhantomData<C>,
}

impl<C: SearchSchemaConfig> FullTextSearchSchema<C> {
    pub fn new(
        weighted_search_fields: HashMap<String, f32>,
        id_fields: HashMap<String, FullTextSearchFieldTypes>,
        boost_factor: f32,
    ) -> Self {
        FullTextSearchSchema {
            weighted_search_fields,
            id_fields,
            boost_factor,
            _marker: std::marker::PhantomData,
        }
    }

    /// Equivalent to calling [`SimpleFullTextSearcher::new`].
    pub fn create_searcher(&self, memory_budget: usize) -> SimpleFullTextSearcher<C> {
        SimpleFullTextSearcher::new(self, memory_budget)
    }

    pub fn create_async_searcher(
        &self,
        memory_budget: usize,
        background: Arc<Background>,
    ) -> AsyncSearcher<C> {
        let simple_searcher = self.create_searcher(memory_budget);
        AsyncSearcher::new(simple_searcher, background)
    }

    #[allow(unused)]
    pub fn add_search_field(&mut self, field_name: String, weight: f32) {
        self.weighted_search_fields.insert(field_name, weight);
    }

    #[allow(unused)]
    pub fn add_id_field(&mut self, field_name: String, field_type: FullTextSearchFieldTypes) {
        self.id_fields.insert(field_name, field_type);
    }
}

pub trait SearchDocumentEntry: FullTextSearchMatchValues {
    fn into_document_entry(self) -> FullTextSearchDocumentEntry;
}

pub trait SearchIdentifyingEntry: FullTextSearchMatchValues {
    fn into_identifying_entry(self) -> FullTextSearchDocumentEntry;
}

pub trait SearchSchemaConfig {
    type SearchDocEntry: SearchDocumentEntry;
    type SearchIdEntry: SearchIdentifyingEntry;
    type SearchHighlight: FullTextSearchMatchHighlights;
}

pub trait ToFieldType {
    fn field_type() -> FullTextSearchFieldTypes;
}

impl ToFieldType for String {
    fn field_type() -> FullTextSearchFieldTypes {
        FullTextSearchFieldTypes::Str
    }
}
impl ToFieldType for usize {
    fn field_type() -> FullTextSearchFieldTypes {
        FullTextSearchFieldTypes::U64
    }
}
impl ToFieldType for u64 {
    fn field_type() -> FullTextSearchFieldTypes {
        FullTextSearchFieldTypes::U64
    }
}
impl ToFieldType for i64 {
    fn field_type() -> FullTextSearchFieldTypes {
        FullTextSearchFieldTypes::I64
    }
}
impl ToFieldType for f64 {
    fn field_type() -> FullTextSearchFieldTypes {
        FullTextSearchFieldTypes::F64
    }
}
impl ToFieldType for bool {
    fn field_type() -> FullTextSearchFieldTypes {
        FullTextSearchFieldTypes::Bool
    }
}

pub trait FromOwnedValue {
    fn from_owned_value(value: OwnedValue) -> Option<Self>
    where
        Self: Sized;
}

impl FromOwnedValue for String {
    fn from_owned_value(value: OwnedValue) -> Option<Self> {
        let OwnedValue::Str(value) = value else {
            return None;
        };
        Some(value)
    }
}
impl FromOwnedValue for usize {
    fn from_owned_value(value: OwnedValue) -> Option<Self> {
        let OwnedValue::U64(value) = value else {
            return None;
        };
        Some(value as usize)
    }
}
impl FromOwnedValue for u64 {
    fn from_owned_value(value: OwnedValue) -> Option<Self> {
        let OwnedValue::U64(value) = value else {
            return None;
        };
        Some(value)
    }
}
impl FromOwnedValue for i64 {
    fn from_owned_value(value: OwnedValue) -> Option<Self> {
        let OwnedValue::I64(value) = value else {
            return None;
        };
        Some(value)
    }
}
impl FromOwnedValue for f64 {
    fn from_owned_value(value: OwnedValue) -> Option<Self> {
        let OwnedValue::F64(value) = value else {
            return None;
        };
        Some(value)
    }
}
impl FromOwnedValue for bool {
    fn from_owned_value(value: OwnedValue) -> Option<Self> {
        let OwnedValue::Bool(value) = value else {
            return None;
        };
        Some(value)
    }
}

#[derive(Debug)]
pub enum SearcherEvent {
    DocumentInserted(FullTextSearchDocumentEntry),
    DocumentDeleted(FullTextSearchDocumentEntry),
    IndexCleared,
}

const SEARCH_ASYNC_BATCH_INTERVAL: Duration = Duration::from_millis(75);
const SEARCH_ASYNC_MAX_BATCH_SIZE: usize = 100;
/// If this amount of time passes without any events, we will join with the
/// index writer (waiting for any remaining operations to complete).
const SEARCH_IDLE_TIMEOUT: Duration = Duration::from_secs(5);

async fn process_searcher_events(
    rx: async_channel::Receiver<SearcherEvent>,
    writer_handle: SearcherWriterHandle,
) {
    let mut running = true;
    while running {
        let mut batch = vec![];

        let mut timer = Timer::never().fuse();
        let mut idle_timer = Timer::at(Instant::now() + SEARCH_IDLE_TIMEOUT).fuse();
        loop {
            futures::select! {
                event = rx.recv().fuse() => {
                    match event {
                        Ok(event) => {
                            if batch.is_empty() {
                                // If we're starting a batch, set a timer to cut off the batch after
                                // a period of time.
                                timer = Timer::at(Instant::now() + SEARCH_ASYNC_BATCH_INTERVAL).fuse();
                                // Unset the idle timer, so that it doesn't interfere with the batch.
                                idle_timer = Timer::never().fuse();
                            }
                            batch.push(event);
                            // If we get a decent batch size, process immediately.
                            if batch.len() >= SEARCH_ASYNC_MAX_BATCH_SIZE {
                                break;
                            }
                        }
                        Err(async_channel::RecvError) => {
                            running = false;
                            break;
                        }
                    }
                },
                _ = timer => {
                    break;
                }
                _ = idle_timer => {
                    // If we hit the idle timeout, and the batch is empty, join with the index writer
                    // threads and terminate them.
                    if batch.is_empty() {
                        if let Err(e) = writer_handle.lock().wait_on_and_drop_indexing_threads() {
                            log::error!("Failed to wait on Tantivy indexing threads: {e:#}");
                        }
                    }
                    break;
                }
            }
        }
        // Process the batch of events.
        if batch.is_empty() {
            continue;
        }
        if let Err(e) = writer_handle.lock().execute_operations(batch) {
            log::error!("Failed to execute search events: {e}");
        }
    }
}

// A wrapper around the searcher to allow for async write operations.
// All search (read) operations remain synchronous and blocking.
pub struct AsyncSearcher<C: SearchSchemaConfig> {
    searcher: SimpleFullTextSearcher<C>,
    tx: async_channel::Sender<SearcherEvent>,
}

impl<C: SearchSchemaConfig> AsyncSearcher<C> {
    fn new(searcher: SimpleFullTextSearcher<C>, background_executor: Arc<Background>) -> Self {
        let (tx, rx) = async_channel::unbounded();
        background_executor
            .spawn(process_searcher_events(rx, searcher.writer.clone()))
            .detach();

        Self { searcher, tx }
    }

    pub fn search_id(
        &self,
        search_term: &str,
    ) -> anyhow::Result<Vec<FullTextSearchMatch<C::SearchIdEntry, C::SearchHighlight>>> {
        self.searcher.search_id(search_term)
    }

    pub fn search_full_doc(
        &self,
        search_term: &str,
    ) -> anyhow::Result<Vec<FullTextSearchMatch<C::SearchDocEntry, C::SearchHighlight>>> {
        self.searcher.search_full_doc(search_term)
    }

    /// Gets all documents in the search index.
    pub fn get_all_documents(&self) -> anyhow::Result<Vec<C::SearchDocEntry>> {
        self.searcher.get_all_documents()
    }

    /// Gets all document identifiers in the search index.
    pub fn get_all_doc_ids(&self) -> anyhow::Result<Vec<C::SearchIdEntry>> {
        self.searcher.get_all_doc_ids()
    }

    // Async write operations
    pub fn clear_search_index_async(&self) -> anyhow::Result<()> {
        block_on(self.tx.send(SearcherEvent::IndexCleared))
            .map_err(|e| anyhow::anyhow!("Failed to send clear index event: {}", e))
    }

    pub fn build_index_async(
        &self,
        documents: impl IntoIterator<Item = C::SearchDocEntry>,
    ) -> anyhow::Result<()> {
        for document in documents {
            self.insert_document_async(document)?;
        }
        Ok(())
    }

    pub fn insert_document_async(&self, entry: C::SearchDocEntry) -> anyhow::Result<()> {
        block_on(
            self.tx
                .send(SearcherEvent::DocumentInserted(entry.into_document_entry())),
        )
        .map_err(|e| anyhow::anyhow!("Failed to send document insertion event: {}", e))
    }

    pub fn delete_document_async(&self, identifying_entry: C::SearchIdEntry) -> anyhow::Result<()> {
        block_on(self.tx.send(SearcherEvent::DocumentDeleted(
            identifying_entry.into_identifying_entry(),
        )))
        .map_err(|e| anyhow::anyhow!("Failed to send document deletion event: {}", e))
    }
}

#[cfg(test)]
#[path = "searcher_tests.rs"]
mod test;
