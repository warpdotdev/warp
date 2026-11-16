[
  (block_scalar)
  ((block_sequence_item) @item @indent (#not-one-line? @item))
  (block_mapping_pair
    key: (_) @key
    !value
  )
  (block_mapping_pair
    key: (_) @key
    value: (_) @val
    (#not-same-line? @key @val)
  )
] @indent