struct Uniforms {
    viewport_size: vec2<f32>,
    // Padding necessary to ensure that the uniforms is 16 bytes. Some wgpu-supported devices (such as webgl) require
    // buffer bindings to be a multiple of 16 bytes.
    padding: vec2<f32>
}

const EPSILON: f32 = 0.0000001;
const PI: f32 = 3.141592653589793;

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct RectVertexShaderInput {
    // The position of the vertex in normalized device coordinates.
    @location(0) vertex_position: vec2<f32>,
    // Bounds of the item in screen coordinates. Origin is contained in `xy`, size is contained in `zw`.
    @location(1) bounds: vec4<f32>,
    @location(2) background_start: vec2<f32>,
    @location(3) background_start_color: vec4<f32>,
    @location(4) background_end: vec2<f32>,
    @location(5) background_end_color: vec4<f32>,
    // Width of the border in the order top, left, right, bottom.
    @location(6) border_width: vec4<f32>,
    @location(7) border_start: vec2<f32>,
    @location(8) border_start_color: vec4<f32>,
    @location(9) border_end: vec2<f32>,
    @location(10) border_end_color: vec4<f32>,
    // Corner radius in the order top_left, top_right, bottom_left, bottom_right.
    @location(11) corner_radius: vec4<f32>,
    // The sigma and padding factor values packed into a single vec2. We pack them together in order
    // to reduce the total number of attributes, which maxes out at 16. See here:
    // https://docs.rs/wgpu/latest/wgpu/struct.Limits.html#structfield.max_vertex_attributes
    @location(12) drop_shadow_data: vec2<f32>,
    // The length of the dash and the gaps for the x and y dimensions, packed into a single vec3.
    @location(13) dashed_border_data: vec3<f32>,
};

struct RectVertexShaderOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) background_start: vec2<f32>,
    @location(1) background_start_color: vec4<f32>,
    @location(2) background_end: vec2<f32>,
    @location(3) background_end_color: vec4<f32>,
    @location(4) border_width: vec4<f32>,
    @location(5) border_start: vec2<f32>,
    @location(6) border_start_color: vec4<f32>,
    @location(7) border_end: vec2<f32>,
    @location(8) border_end_color: vec4<f32>,
    @location(9) rect_corner: vec2<f32>,
    @location(10) rect_center: vec2<f32>,
    @location(11) corner_radius: vec4<f32>,
    @location(12) drop_shadow_data: vec2<f32>,
    @location(13) dashed_border_data: vec3<f32>,
};

@vertex
fn vs_main(
    in: RectVertexShaderInput,
) -> RectVertexShaderOutput {
    var out: RectVertexShaderOutput;
    var origin: vec2<f32> = in.bounds.xy;
    var size: vec2<f32> = in.bounds.zw;
    var pixel_pos: vec2<f32> = in.vertex_position * size + origin;
    // Convert the position of the item from screen coordinates into normalized device coordinates
    var ndc_position: vec2<f32> = pixel_pos / uniforms.viewport_size * vec2(2.0, -2.0) + vec2(-1.0, 1.0);

    out.position = vec4<f32>(ndc_position, 0.0, 1.0);
    out.background_start = in.background_start * size + origin;
    out.background_start_color = in.background_start_color;
    out.background_end = in.background_end * size + origin;
    out.background_end_color = in.background_end_color;
    out.border_start = in.border_start * size + origin;
    out.border_start_color = in.border_start_color;
    out.border_end = in.border_end * size + origin;
    out.border_end_color = in.border_end_color;
    out.border_width = in.border_width;
    out.corner_radius = in.corner_radius;
    out.rect_corner = size / 2.;
    out.rect_center = origin + out.rect_corner;
    out.drop_shadow_data = in.drop_shadow_data;
    out.dashed_border_data = in.dashed_border_data;

    return out;
}

@fragment
fn rect_fs_main(in: RectVertexShaderOutput) -> @location(0) vec4<f32> {
    var background_color: vec4<f32> = derive_color(
        in.position.xy,
        in.background_start,
        in.background_end,
        in.background_start_color,
        in.background_end_color
    );
    var border_color: vec4<f32> = derive_color(
        in.position.xy,
        in.border_start,
        in.border_end,
        in.border_start_color,
        in.border_end_color
    );

    // There are actually two different radii at play here - the inner
    // (background) and outer (shape) radii.  The inner radius is equal to the
    // outer radius minus the border width, in order for the two curves to
    // maintain a constant distance from each other.
    var inner_corner_radius: f32;
    var outer_corner_radius: f32;

    var border_inner_corner: vec2<f32> = in.rect_corner;
    if in.position.y >= in.rect_center.y {
        // Bottom half
        border_inner_corner.y -= in.border_width.z;
        if in.position.x >= in.rect_center.x {
          // Bottom right quadrant
            border_inner_corner.x -= in.border_width.y;
            outer_corner_radius = in.corner_radius.w;
            inner_corner_radius = max(0.0, outer_corner_radius - in.border_width.z);
        } else {
          // Bottom left quadrant
            border_inner_corner.x -= in.border_width.w;
            outer_corner_radius = in.corner_radius.z;
            inner_corner_radius = max(0.0, outer_corner_radius - in.border_width.z);
        }
    } else {
        // Top half
        border_inner_corner.y -= in.border_width.x;
        if in.position.x >= in.rect_center.x {
          // Top right quadrant
            border_inner_corner.x -= in.border_width.y;
            outer_corner_radius = in.corner_radius.y;
            inner_corner_radius = max(0.0, outer_corner_radius - in.border_width.x);
        } else {
          // Top left quadrant
            border_inner_corner.x -= in.border_width.w;
            outer_corner_radius = in.corner_radius.x;
            inner_corner_radius = max(0.0, outer_corner_radius - in.border_width.x);
        }
    }

    var rect_origin: vec2<f32> = in.rect_center - in.rect_corner;
    var outer_distance: f32 = distance_from_rect(in.position.xy, in.rect_center, in.rect_corner, outer_corner_radius);
    var inner_distance: f32 = distance_from_rect(in.position.xy, in.rect_center, border_inner_corner, inner_corner_radius);

    var drop_shadow_sigma = in.drop_shadow_data.x;
    var drop_shadow_padding_factor = in.drop_shadow_data.y;
    if drop_shadow_sigma > 0.0 {
        var rect_size: vec2<f32> = in.rect_corner * 2.0;
        // When we are rendering a drop shadow we need to pass in the positions
        // of the original rect, so we figure them out from the padding.
        // Note we subtract twice the padding, because the padding is specified
        // in terms of padding on a single side.
        var shadowed_rect_origin: vec2<f32> = rect_origin + drop_shadow_padding_factor;
        var shadowed_rect_size: vec2<f32> = rect_size - 2.0 * drop_shadow_padding_factor;
        background_color.a *= rounded_box_shadow(
            shadowed_rect_origin,
            shadowed_rect_origin + shadowed_rect_size,
            in.position.xy,
            drop_shadow_sigma,
            outer_corner_radius
        );
    } else {
        // Adjust the opacity of the border color based on where the pixel lies
        // between the background and the border_width.
        border_color.a *= saturate(inner_distance + 0.5);

        // Force the alpha value to 0 (fully transparent) if the pixel is
        // outside the border_width.
        //
        // When we are outside the border, outer_distance is a larger positive
        // value than inner_distance.  When we are inside the border itself,
        // outer_distance is negative and inner_distance is positive.  When we
        // are inside the inner border edge, outer_distance is more negative
        // than inner_distance.
        border_color.a *= f32(inner_distance > outer_distance);

        var rect_bottom_right = in.rect_center + in.rect_corner;
        var pos_from_origin = in.position.xy - rect_origin;

        // Masks for pixels outside of inner rectangle or on border
        var is_horizontal_border = (in.position.y <= rect_origin.y + in.border_width.x) || (in.position.y >= rect_bottom_right.y - in.border_width.z);
        var is_vertical_border = (in.position.x <= rect_origin.x + in.border_width.w) || (in.position.x >= rect_bottom_right.x - in.border_width.y);

        var dash_length = in.dashed_border_data.x;
        var gap_lengths = in.dashed_border_data.yz;

        // Get length along the dash and gap segment and determine if pixel is in dash or gap
        var length_on_dash_and_gap_segment_x = pos_from_origin.x % (dash_length + gap_lengths.x);
        var length_on_dash_and_gap_segment_y = pos_from_origin.y % (dash_length + gap_lengths.y);
        var is_horizontal_dash = is_horizontal_border && (length_on_dash_and_gap_segment_x < dash_length);
        var is_vertical_dash = is_vertical_border && (length_on_dash_and_gap_segment_y < dash_length);

        // Mask out any gaps in the border
        border_color.a *= f32(dash_length <= 0.0 || is_horizontal_dash || is_vertical_dash);

        // Perform proper alpha blending on the two colors, avoiding a
        // divide-by-zero if both colors are fully transparent.
        //
        // See formula for "over" compositing here: https://en.wikipedia.org/wiki/Alpha_compositing#Alpha_blending
        var alpha: f32 = border_color.a + background_color.a * (1.0 - border_color.a);
        var new_background_color: vec3<f32> = (border_color.rgb * border_color.a + background_color.rgb * background_color.a * (1.0 - border_color.a)) / (alpha + EPSILON);
        background_color = vec4(new_background_color, alpha);
    }

    // If there's a corner radius we need to do some anti aliasing to smooth out the rounded corner effect.
    if outer_corner_radius > 0. {
        background_color.a *= 1.0 - saturate(outer_distance + 0.5);
    }

    return background_color;
}

fn derive_color(
    position: vec2<f32>,
    start: vec2<f32>,
    end: vec2<f32>,
    start_color: vec4<f32>,
    end_color: vec4<f32>
) -> vec4<f32> {
    var adjusted_end: vec2<f32> = end - start;
    var h: f32 = dot(position - start, adjusted_end) / dot(adjusted_end, adjusted_end);
    return mix(start_color, end_color, h);
}

// Based on the fragement position and the center of the quad, select one of the 4 radi.
// Order matches CSS border radius attribute:
// radi.x = top-left, radi.y = top-right, radi.z = bottom-right, radi.w = bottom-left
fn select_border_radius(radi: vec4<f32>, position: vec2<f32>, center: vec2<f32>) -> f32 {
    var rx = radi.x;
    var ry = radi.y;
    rx = select(radi.x, radi.y, position.x > center.x);
    ry = select(radi.w, radi.z, position.x > center.x);
    rx = select(rx, ry, position.y > center.y);
    return rx;
}

fn distance_from_rect(pixel_pos: vec2<f32>, rect_center: vec2<f32>, rect_corner: vec2<f32>, corner_radius: f32) -> f32 {
    var p: vec2<f32> = pixel_pos - rect_center;
    var q: vec2<f32> = abs(p) - rect_corner + corner_radius;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - corner_radius;
}

// Drop shadow code *heavily* inspired by this post:
// http://madebyevan.com/shaders/fast-rounded-rectangle-shadows/

// Return the mask for the shadow of a box from lower to upper
fn rounded_box_shadow(lower: vec2<f32>, upper: vec2<f32>, in_point: vec2<f32>, sigma: f32, corner: f32) -> f32 {
    // Center everything to make the math easier
    var center: vec2<f32> = (lower + upper) * 0.5;
    var half_size: vec2<f32> = (upper - lower) * 0.5;
    var point = in_point - center;

    // The signal is only non-zero in a limited range, so don't waste samples
    var low: f32 = point.y - half_size.y;
    var high: f32 = point.y + half_size.y;
    var start: f32 = clamp(-3.0 * sigma, low, high);
    var end: f32 = clamp(3.0 * sigma, low, high);

    // Accumulate samples (we can get away with surprisingly few samples)
    var step: f32 = (end - start) / 4.0;
    var y: f32 = start + step * 0.5;
    var value: f32 = 0.0;
    for (var i = 0; i < 4; i++) {
        value += rounded_box_shadow_x(point.x, point.y - y, sigma, corner, half_size) * gaussian(y, sigma) * step;
        y += step;
    }

    return value;
}

// Return the blurred mask along the x dimension
fn rounded_box_shadow_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2<f32>) -> f32 {
    var delta: f32 = min(half_size.y - corner - abs(y), 0.0);
    var curved: f32 = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
    var integral: vec2<f32> = 0.5 + 0.5 * erf((x + vec2(-curved, curved)) * (sqrt(0.5) / sigma));
    return integral.y - integral.x;
}

// This approximates the error function, needed for the gaussian integral
fn erf(x: vec2<f32>) -> vec2<f32> {
    var s = sign(x);
    var a = abs(x);
    var denom = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    denom *= denom;
    return s - s / (denom * denom);
}

// A standard gaussian function, used for weighting samples
fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * PI) * sigma);
}
