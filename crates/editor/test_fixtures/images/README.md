# Test Images

This directory contains test images for validating markdown image rendering.

## Files

- `image_test.md` - Markdown file with various image references
- `sample1.jpg` - 400x300 JPEG with vertical gradient (blue to red-orange) and white border
- `sample2.jpg` - 600x400 JPEG with checkered pattern and orange circle overlay
- `sample3.png` - 300x300 PNG with radial pattern and transparency (alpha channel)
- `parent_test.jpg` - 300x200 JPEG with diagonal blue stripes, located in parent directory for testing relative path resolution

## Purpose

These images test different aspects of image rendering:
- Different formats (JPEG, PNG)
- Different dimensions (300x300, 400x300, 600x400)
- Transparency (PNG with alpha channel)
- Visual patterns to verify proper rendering (gradients, patterns, shapes)
- Relative path resolution (parent directory references)
