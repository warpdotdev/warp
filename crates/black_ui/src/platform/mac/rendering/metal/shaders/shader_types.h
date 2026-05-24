#ifndef shader_types_h
#define shader_types_h

#include <simd/simd.h>

typedef struct {
  vector_float2 viewport_size;
} Uniforms;

typedef struct {
  vector_float2 origin;
  vector_float2 size;
  float corner_radius_top_left;
  float corner_radius_top_right;
  float corner_radius_bottom_left;
  float corner_radius_bottom_right;
  float border_top;
  float border_right;
  float border_bottom;
  float border_left;
  vector_float2 background_start;
  vector_float2 background_end;
  vector_float4 background_start_color;
  vector_float4 background_end_color;
  vector_float2 border_start;
  vector_float2 border_end;
  vector_float4 border_start_color;
  vector_float4 border_end_color;
  vector_float4 icon_color;
  int is_icon;
  vector_float2 drop_shadow_offsets;
  vector_float4 drop_shadow_color;
  float drop_shadow_sigma;
  float drop_shadow_padding_factor;
  float dash_length;
  vector_float2 gap_lengths;
} PerRectUniforms;

typedef struct {
  vector_float2 origin;
  vector_float2 size;
  float uv_left;
  float uv_top;
  float uv_width;
  float uv_height;
  float fade_start;
  float fade_end;
  vector_float4 color;
  int is_emoji;
} PerGlyphUniforms;

#endif // shader_types_h
