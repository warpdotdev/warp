/// Converts Rust types to FullTextSearchFieldTypes variants
#[macro_export]
macro_rules! type_to_field_type {
    ($t:ty) => {
        <$t as $crate::searcher::ToFieldType>::field_type()
    };
}
pub use type_to_field_type;

#[macro_export]
macro_rules! data_from_owned_value {
    ($value:expr, $t:ty) => {
        <$t as $crate::searcher::FromOwnedValue>::from_owned_value($value)
    };
}

#[macro_export]
macro_rules! get_factor_or_default {
    ($factor:expr) => {
        $factor
    };
    () => {
        1.0
    };
}
pub use get_factor_or_default;

/// Macro to define a search schema for a [`crate::searcher::SimpleFullTextSearcher`].
/// ### Parameters
/// * `schema_name` - The name of the schema. This would be the name of the static reference of the schema.
/// * `config_name` - The name of the generated type config corresponding to the defined schema.
/// * `search_doc` - The name of the search document struct. This is the type that the searcher expects when you insert
///   documents into the search index. This struct contains all the fields defined in the schema (both the search and
///   id fields).
/// * `identifying_doc` - The name of the identifying document struct. This is the type that the searcher expects when you
///   attempt to delete documents from the search index. This struct contains only the id fields defined in the schema.
///   It is expected that all the id fields in combination uniquely identify a document.
/// * `search_fields` - A list of fields that are searchable. Each field is a tuple of the field name and the weight.
///   The weight is used to determine the relevance of the field when searching. The higher the weight, the more relevant.
///   Note that the weights do not need to add up to 1 and will be normalized by the searcher.
/// * `id_fields` - A list of fields that are used to identify the document. These fields are not searchable and are the
///   "data" associated with the document. **It is expected that all the id fields together forms a uniquely-identifying key
///   of a document!** Failure to do so will result in unexpected behaviour when inserting and deleting documents.
/// ## Defining a new search schema
/// Here is an example of using this schema to create a simple searcher:
/// ```
/// use itertools::Itertools;
/// use warp_search_core::define_search_schema;
/// use warp_search_core::searcher::{SimpleFullTextSearcher, DEFAULT_MEMORY_BUDGET};
///
/// define_search_schema!(
///     schema_name: MY_SCHEMA,
///     config_name: MyConfig,
///     search_doc: MySearchDoc,
///     identifying_doc: MyIdDoc,
///     search_fields: [name: 1.0, description: 0.5],
///     id_fields: [id: u64]
/// );
///
/// struct SearchWrapper {
///     searcher: SimpleFullTextSearcher<MyConfig>,
/// }
///
/// struct SearchResult {
///     doc_id: usize,
///     /// Byte indices of highlighted matches in the name field.
///     name_highlights: Vec<usize>,
///     /// Byte indices of highlighted matches in the description field.
///     description_highlights: Vec<usize>,
///     /// Relevance score of the match.
///     score: f64,
/// }
///
/// impl SearchWrapper {
///     fn new(initial_index: impl IntoIterator<Item = (String, String, u64)>) -> anyhow::Result<Self> {
///         let searcher = MY_SCHEMA.create_searcher(DEFAULT_MEMORY_BUDGET);
///         searcher.build_index(initial_index.into_iter().map(|(name, description, id)| {
///             MySearchDoc { name, description, id }
///         }))?;
///
///         Ok(Self { searcher })
///     }
///
///     fn add_document(&mut self, name: String, description: String, id: u64) -> anyhow::Result<()> {
///         self.searcher.insert_document(MySearchDoc { name, description, id })
///     }
///
///     fn remove_document_by_id(&mut self, id: u64) -> anyhow::Result<()> {
///         self.searcher.delete_document(MyIdDoc { id })
///     }
///
///     fn search(&self, query: &str) -> anyhow::Result<Vec<SearchResult>> {
///         Ok(self.searcher
///             .search_full_doc(query)?
///             .into_iter()
///             .map(|match_result| {
///                 SearchResult {
///                     doc_id: match_result.values.id as usize,
///                     name_highlights: match_result.highlights.name,
///                     description_highlights: match_result.highlights.description,
///                     score: match_result.score,
///                 }
///             })
///             .sorted_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
///             .collect())
///     }
/// }
/// ```
#[macro_export]
macro_rules! define_search_schema {
    (schema_name: $schema_name:ident, config_name: $config_name:ident, search_doc: $search_doc:ident, identifying_doc: $id_doc_name:ident, search_fields: [$($s_name:ident: $weight:literal$(,)?)*], id_fields: [$($i_name:ident: $value_type:ty$(,)?)*] $(, boost_factor: $boost_factor:expr)? $(,)?) => {
        lazy_static::lazy_static! {
            static ref $schema_name: $crate::searcher::FullTextSearchSchema<$config_name> = $crate::searcher::FullTextSearchSchema::new(
                std::collections::HashMap::from([
                    $((stringify!($s_name).to_owned(), $weight)),*
                ]),
                std::collections::HashMap::from([
                    $((stringify!($i_name).to_owned(), $crate::type_to_field_type!($value_type))),*
                ]),
                $crate::get_factor_or_default!($($boost_factor)*),
            );
        }

        #[derive(Debug, Clone)]
        struct $search_doc {
            $(
                pub $s_name: String,
            )*
            $(
                pub $i_name: $value_type
            ),*
        }

        impl $crate::searcher::SearchDocumentEntry for $search_doc {
            fn into_document_entry(self) -> $crate::searcher::FullTextSearchDocumentEntry {
                let mut entry = std::collections::HashMap::new();
                $(
                    entry.insert(
                        stringify!($s_name).to_owned(),
                        self.$s_name.into(),
                    );
                )*
                $(
                    entry.insert(
                        stringify!($i_name).to_owned(),
                        self.$i_name.into(),
                    );
                )*
                entry
            }
        }

        impl $crate::searcher::FullTextSearchMatchValues for $search_doc {
            fn from_match_result_values(mut values: std::collections::HashMap<String, $crate::tantivy::schema::OwnedValue>) -> Option<Self> {
                Some(Self {
                    $(
                        $s_name: $crate::data_from_owned_value!(values.remove(stringify!($s_name))?, String)?,
                    )*
                    $(
                        $i_name: $crate::data_from_owned_value!(values.remove(stringify!($i_name))?, $value_type)?
                    ),*
                })
            }
        }

        #[allow(unused)]
        #[derive(Debug, Clone)]
        struct $id_doc_name {
            $(
                pub $i_name: $value_type
            ),*
        }

        #[allow(unused)]
        impl $crate::searcher::SearchIdentifyingEntry for $id_doc_name {
            fn into_identifying_entry(self) -> $crate::searcher::FullTextSearchDocumentEntry {
                let mut entry = std::collections::HashMap::new();
                $(
                    entry.insert(
                        stringify!($i_name).to_owned(),
                        self.$i_name.into(),
                    );
                )*
                entry
            }
        }

        #[allow(unused)]
        impl $crate::searcher::FullTextSearchMatchValues for $id_doc_name {
            fn from_match_result_values(mut values: std::collections::HashMap<String, $crate::tantivy::schema::OwnedValue>) -> Option<Self> {
                Some(Self {
                    $(
                        $i_name: $crate::data_from_owned_value!(values.remove(stringify!($i_name))?, $value_type)?,
                    )*
                })
            }
        }

        $crate::paste::paste! {
            #[allow(unused)]
            #[derive(Debug, Clone)]
            struct [<_ $config_name HighlightResult>] {
                $(
                    pub $s_name: Vec<usize>
                ),*
            }

            #[allow(unused)]
            impl $crate::searcher::FullTextSearchMatchHighlights for [<_ $config_name HighlightResult>] {
                fn from_match_result_highlights(mut highlights: std::collections::HashMap<String, Vec<usize>>) -> Option<Self> {
                    Some(Self {
                        $(
                            $s_name: highlights.remove(stringify!($s_name))?,
                        )*
                    })
                }
            }

            struct $config_name;
            impl $crate::searcher::SearchSchemaConfig for $config_name {
                type SearchDocEntry = $search_doc;
                type SearchIdEntry = $id_doc_name;
                type SearchHighlight = [<_ $config_name HighlightResult>];
            }
        }
    };
}
pub use define_search_schema;
