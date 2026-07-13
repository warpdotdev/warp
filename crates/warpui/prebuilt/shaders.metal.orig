#include <metal_stdlib>

using namespace metal;

#include "shader_types.h"

constant float EPSILON = 0.00001;

// Vertex shader outputs and fragment shader inputs
struct RectFragmentData
{
    float4 position [[position]];
    float2 pixel_position [[pixel_position]];
    float2 rect_origin;
    float2 rect_size;
    float2 rect_center;
    float2 rect_corner;
    float border_top;
    float border_right;
    float border_bottom;
    float border_left;
    float corner_radius_top_left;
    float corner_radius_top_right;
    float corner_radius_bottom_left;
    float corner_radius_bottom_right;
    float2 background_start;
    float2 background_end;
    float4 background_start_color;
    float4 background_end_color;
    float2 border_start;
    float2 border_end;
    float4 border_start_color;
    float4 border_end_color;
    float2 texture_coordinate;
    bool is_icon;
    float4 icon_color;
    float2 drop_shadow_offsets;
    float4 drop_shadow_color;
    float drop_shadow_sigma;
    float drop_shadow_padding_factor;
    float dash_length;
    float2 gap_lengths;
};

struct GlyphFragmentData
{
    float4 position [[position]];
    float2 rect_center;
    float2 rect_corner;
    float2 texture_coordinate;
    float fade_alpha;
    float4 color;
    bool is_emoji;
};


float distance_from_rect(vector_float2 pixel_pos, vector_float2 rect_center, vector_float2 rect_corner, float corner_radius) {
    vector_float2 p = pixel_pos - rect_center;
    vector_float2 q = abs(p) - rect_corner + corner_radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - corner_radius;
}

float4 derive_color(float2 pixel_pos, float2 start, float2 end, float4 start_color, float4 end_color) {
    float2 adjusted_end = end - start;
    float h = dot(pixel_pos - start, adjusted_end) / dot(adjusted_end, adjusted_end);
    return mix(start_color, end_color, h);
}

vertex RectFragmentData
rect_vertex_shader(
    uint vertex_id [[vertex_id]],
    uint instance_id [[instance_id]],
    constant float2 *vertices [[buffer(0)]],
    constant PerRectUniforms *glyph_uniforms [[buffer(1)]],
    constant Uniforms *uniforms [[buffer(2)]])
{
    const constant PerRectUniforms *rect = &glyph_uniforms[instance_id];

    float2 pixel_pos = vertices[vertex_id] * rect->size + rect->origin;
    float2 device_pos = pixel_pos / uniforms->viewport_size * float2(2.0, -2.0) + float2(-1.0, 1.0);

    RectFragmentData out;
    out.position = float4(device_pos, 0.0, 1.0);
    out.pixel_position = pixel_pos;
    out.rect_origin = rect->origin;
    out.rect_size = rect->size;
    out.rect_corner = rect->size / 2.0;
    out.rect_center = rect->origin + out.rect_corner;
    out.border_top = rect->border_top;
    out.border_right = rect->border_right;
    out.border_bottom = rect->border_bottom;
    out.border_left = rect->border_left;
    out.corner_radius_top_left = rect->corner_radius_top_left;
    out.corner_radius_top_right = rect->corner_radius_top_right;
    out.corner_radius_bottom_left = rect->corner_radius_bottom_left;
    out.corner_radius_bottom_right = rect->corner_radius_bottom_right;
    out.background_start = rect->background_start * rect->size + rect->origin;
    out.background_end = rect->background_end * rect->size + rect->origin;
    out.background_start_color = rect->background_start_color;
    out.background_end_color = rect->background_end_color;
    out.border_start = rect->border_start * rect->size + rect->origin;
    out.border_end = rect->border_end * rect->size + rect->origin;
    out.border_start_color = rect->border_start_color;
    out.border_end_color = rect->border_end_color;
    out.texture_coordinate = vertices[vertex_id];
    out.is_icon = rect->is_icon;
    out.icon_color = rect->icon_color;
    out.drop_shadow_offsets = rect->drop_shadow_offsets;
    out.drop_shadow_color = rect->drop_shadow_color;
    out.drop_shadow_sigma = rect->drop_shadow_sigma;
    out.drop_shadow_padding_factor = rect->drop_shadow_padding_factor;
    out.dash_length = rect->dash_length;
    out.gap_lengths = rect->gap_lengths;
    return out;
}

// Drop shadow code *heavily* inspired by this post:
// http://madebyevan.com/shaders/fast-rounded-rectangle-shadows/

// A standard gaussian function, used for weighting samples
float gaussian(float x, float sigma) {
  const float pi = 3.141592653589793;
  return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * pi) * sigma);
}

// This approximates the error function, needed for the gaussian integral
float2 erf(float2 x) {
  float2 s = sign(x), a = abs(x);
  x = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
  x *= x;
  return s - s / (x * x);
}

// Return the blurred mask along the x dimension
float roundedBoxShadowX(float x, float y, float sigma, float corner, float2 halfSize) {
  float delta = min(halfSize.y - corner - abs(y), 0.0);
  float curved = halfSize.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
  float2 integral = 0.5 + 0.5 * erf((x + float2(-curved, curved)) * (sqrt(0.5) / sigma));
  return integral.y - integral.x;
}

// Return the mask for the shadow of a box from lower to upper
float roundedBoxShadow(float2 lower, float2 upper, float2 point, float sigma, float corner) {
  // Center everything to make the math easier
  float2 center = (lower + upper) * 0.5;
  float2 halfSize = (upper - lower) * 0.5;
  point -= center;

  // The signal is only non-zero in a limited range, so don't waste samples
  float low = point.y - halfSize.y;
  float high = point.y + halfSize.y;
  float start = clamp(-3.0 * sigma, low, high);
  float end = clamp(3.0 * sigma, low, high);

  // Accumulate samples (we can get away with surprisingly few samples)
  float step = (end - start) / 4.0;
  float y = start + step * 0.5;
  float value = 0.0;
  for (int i = 0; i < 4; i++) {
    value += roundedBoxShadowX(point.x, point.y - y, sigma, corner, halfSize) * gaussian(y, sigma) * step;
    y += step;
  }

  return value;
}

fragment float4 rect_fragment_shader(
    RectFragmentData in [[stage_in]],
    constant Uniforms *uniforms [[buffer(0)]])
{
    float outer_distance;
    float inner_distance;
    // There are actually two different radii at play here - the inner
    // (background) and outer (shape) radii.  The inner radius is equal to the
    // outer radius minus the border width, in order for the two curves to
    // maintain a constant distance from each other.
    float outer_corner_radius;
    float inner_corner_radius;

    // Length along the perimeter of (rounded) rectangle, starting from top left.
    float length_along = 0.;
    float2 pos_from_origin = in.position.xy - in.rect_origin;

    float2 border_inner_corner = in.rect_corner;
    if (in.position.y >= in.rect_center.y) {
        // Bottom half
        border_inner_corner.y -= in.border_bottom;
        if (in.position.x >= in.rect_center.x) {
            // Bottom right quadrant
            border_inner_corner.x -= in.border_right;
            outer_corner_radius = in.corner_radius_bottom_right;
            inner_corner_radius = max(0.0, outer_corner_radius - in.border_bottom);
        } else {
            // Bottom left quadrant
            border_inner_corner.x -= in.border_left;
            outer_corner_radius = in.corner_radius_bottom_left;
            inner_corner_radius = max(0.0, outer_corner_radius - in.border_bottom);
        }
    } else {
        // Top half
        border_inner_corner.y -= in.border_top;
        if (in.position.x >= in.rect_center.x) {
            // Top right quadrant
            border_inner_corner.x -= in.border_right;
            outer_corner_radius = in.corner_radius_top_right;
            inner_corner_radius = max(0.0, outer_corner_radius - in.border_top);
        } else {
            // Top left quadrant
            border_inner_corner.x -= in.border_left;
            outer_corner_radius = in.corner_radius_top_left;
            inner_corner_radius = max(0.0, outer_corner_radius - in.border_top);
        }
    }

    float2 rect_bottom_right = in.rect_origin + in.rect_size;

    outer_distance = distance_from_rect(in.position.xy, in.rect_center, in.rect_corner, outer_corner_radius);
    inner_distance = distance_from_rect(in.position.xy, in.rect_center, border_inner_corner, inner_corner_radius);

    float4 color;
    if (in.drop_shadow_sigma > 0) {
        color = in.drop_shadow_color;
        // When we are rendering a drop shadow we need to pass in the positions
        // of the original rect, so we figure them out from the padding.
        // Note we subtract twice the padding, because the padding is specified
        // in terms of padding on a single side.
        float2 shadowed_rect_origin = in.rect_origin + in.drop_shadow_padding_factor;
        float2 shadowed_rect_size = in.rect_size - 2 * in.drop_shadow_padding_factor;
        color.a *= roundedBoxShadow(
                    shadowed_rect_origin,
                    shadowed_rect_origin + shadowed_rect_size,
                    in.pixel_position,
                    in.drop_shadow_sigma,
                    outer_corner_radius);
    } else {
        // Solid fill case (not a drop shadow)
        float4 background_color = derive_color(in.position.xy, in.background_start, in.background_end, in.background_start_color, in.background_end_color);
        float4 border_color = derive_color(in.position.xy, in.border_start, in.border_end, in.border_start_color, in.border_end_color);

        // Adjust the opacity of the border color based on where the pixel lies
        // between the background and the border.
        border_color.a *= saturate(inner_distance + 0.5);

        // Force the alpha value to 0 (fully transparent) if the pixel is
        // outside the border.
        //
        // When we are outside the border, outer_distance is a larger positive
        // value than inner_distance.  When we are inside the border itself,
        // outer_distance is negative and inner_distance is positive.  When we
        // are inside the inner border edge, outer_distance is more negative
        // than inner_distance.
        border_color.a *= inner_distance > outer_distance;

        // Masks for pixels outside of inner rectangle or on border
        bool is_horizontal_border = (in.position.y <= in.rect_origin.y + in.border_top) || (in.position.y >= rect_bottom_right.y - in.border_bottom);
        bool is_vertical_border = (in.position.x <= in.rect_origin.x + in.border_left) || (in.position.x >= rect_bottom_right.x - in.border_right);

        // Get length along the dash and gap segment and determine if pixel is in dash or gap
        float length_on_dash_and_gap_segment_x = fmod(pos_from_origin.x, in.dash_length + in.gap_lengths.x);
        float length_on_dash_and_gap_segment_y = fmod(pos_from_origin.y, in.dash_length + in.gap_lengths.y);
        bool is_horizontal_dash = is_horizontal_border && (length_on_dash_and_gap_segment_x < in.dash_length);
        bool is_vertical_dash = is_vertical_border && (length_on_dash_and_gap_segment_y < in.dash_length);

        // Mask out any gaps in the border
        border_color.a *= in.dash_length <= 0 || (is_horizontal_dash || is_vertical_dash);

        // Perform proper alpha blending on the two colors, avoiding a
        // divide-by-zero if both colors are fully transparent.
        //
        // See formula for "over" compositing here: https://en.wikipedia.org/wiki/Alpha_compositing#Alpha_blending
        float alpha = border_color.a + background_color.a * (1.0 - border_color.a);
        color.rgb = (border_color.rgb * border_color.a + background_color.rgb * background_color.a * (1.0 - border_color.a)) / (alpha + EPSILON);
        color.a = alpha;
    }

    // If there's a corner radius we need to do some anti aliasing to smooth out the rounded corner effect.
    if (outer_corner_radius > 0) {
        color.a *= 1.0 - saturate(outer_distance + 0.5);
    }

    return color;
}

fragment float4 image_fragment_shader(
    RectFragmentData in [[stage_in]],
    texture2d<half> color_texture [[ texture(0) ]])
{
    constexpr sampler texture_sampler (mag_filter::linear,
                                       min_filter::linear);

    // Sample the texture to obtain a color
    const half4 color_sample = color_texture.sample(texture_sampler, in.texture_coordinate);

    float4 color;
    // If the image is an icon, use the provided icon_color instead of sampling from texture
    if (in.is_icon) {
        vector_float4 in_color = in.icon_color;
        in_color.a *= color_sample.r;
        color = float4(in_color);
    } else {
        color = float4(color_sample);
        color.a *= in.icon_color.a;
    }

    float outer_corner_radius;

    if (in.position.y >= in.rect_center.y) {
        // Bottom half
        if (in.position.x >= in.rect_center.x) {
            // Bottom right quadrant
            outer_corner_radius = in.corner_radius_bottom_right;
        } else {
            // Bottom left quadrant
            outer_corner_radius = in.corner_radius_bottom_left;
        }
    } else {
        // Top half
        if (in.position.x >= in.rect_center.x) {
            // Top right quadrant
            outer_corner_radius = in.corner_radius_top_right;
        } else {
            // Top left quadrant
            outer_corner_radius = in.corner_radius_top_left;
        }
    }

    float outer_distance = distance_from_rect(in.position.xy, in.rect_center, in.rect_corner, outer_corner_radius);

    // If there's a corner radius we need to do some anti aliasing to smooth out the rounded corner effect.
    if (outer_corner_radius > 0) {
        color.a *= 1.0 - saturate(outer_distance + 0.5);
    }
    return color;
}

vertex GlyphFragmentData
glyph_vertex_shader(
        uint vertex_id [[vertex_id]],
        uint instance_id [[instance_id]],
        constant vector_float2 *vertices [[buffer(0)]],
        const device PerGlyphUniforms *glyph_uniforms [[buffer(1)]],
        constant Uniforms *uniforms [[buffer(2)]])
{
    const device PerGlyphUniforms *glyph = &glyph_uniforms[instance_id];

    float2 pixel_pos = vertices[vertex_id] * glyph->size + glyph->origin;
    // Use floor here to vertically align the glyph to the pixel grid.
    // If it's not aligned to the grid, the fragment shader will do its
    // own interpolation, which makes it so we don't use the anti-aliasing
    // from core text, which is what we want.  We don't force the glyph to a
    // horizontal pixel position because we rasterize the glyph at multiple
    // subpixel positions, and so the very slight linear interpolation here
    // won't produce a fuzzy glyph, just a correctly-positioned one.
    pixel_pos = float2(pixel_pos.x, floor(pixel_pos.y));

    // Evaluating the glyphs fade effect. Note that the fade may go in two different directions:
    // - Right to left (default) - where the opaque side is on the right, and transparent on the left
    //   (in this case, the start_fade < end_fade; start is where the fade is transparent)
    // - Left to right - where the opaque side is on the left, and it fades towards the right side.
    //   In this case, start_fade > end_fade, and the opaque side is on the left (end_fade).
    // To clarify: fade_start is ALWAYS where the fade is transparent, and fade_end is ALWAYS where
    // the opaque part is, this is reflected in how we compute width, dist, and alpha.
    float fade_width = fabs(glyph->fade_end - glyph->fade_start);
    float fade_dist = pixel_pos.x - fmin(glyph->fade_start, glyph->fade_end);

    float fade_alpha;
    if (glyph->fade_end < glyph->fade_start) { // left-to-right case
      fade_alpha = fade_dist / fade_width;
    } else { // right-to-left case
      fade_alpha = 1 - fade_dist / fade_width;
    }

    vector_float2 device_pos = pixel_pos / uniforms->viewport_size * vector_float2(2.0, -2.0) + vector_float2(-1.0, 1.0);

    vector_float2 texture_coordinate  = vector_float2(glyph->uv_left, glyph->uv_top) + vertices[vertex_id] * vector_float2(glyph->uv_width, glyph->uv_height);

    GlyphFragmentData out;
    out.position = vector_float4(device_pos, 0.0, 1.0);
    out.rect_corner = glyph->size / 2.0;
    out.rect_center = glyph->origin + out.rect_corner;
    out.texture_coordinate = texture_coordinate;
    out.fade_alpha = fade_alpha;
    out.color = glyph->color;
    out.is_emoji = glyph->is_emoji;
    return out;
}

fragment float4 glyph_fragment_shader(
    GlyphFragmentData in [[stage_in]],
    texture2d<half> color_texture [[ texture(0) ]]
) {
    // Sample the texture to obtain a color.
    constexpr sampler texture_sampler (mag_filter::linear, min_filter::linear);
    const float4 color_sample = float4(color_texture.sample(texture_sampler, in.texture_coordinate));
    // Use the input color for non-emoji, and the sampled color for emoji.
    float4 color = mix(in.color, color_sample, float(in.is_emoji));
    // Multiply alpha by the sampled color's red channel for non-emoji.
    color.a *= max(color_sample.r, float(in.is_emoji));
    // Apply the fade.
    color.a *= saturate(in.fade_alpha);
    return color;
}
