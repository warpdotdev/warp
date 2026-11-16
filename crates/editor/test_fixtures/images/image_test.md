# Markdown Image Test

This file tests various ways to include images in markdown.

## Sample JPG Images

Here's a 400x300 sample image:

![Sample Image 1](./sample1.jpg)

Here's a larger 600x400 sample:

![Sample Image 2](./sample2.jpg)

## PNG Image

A square 300x300 PNG:

![Sample PNG](./sample3.png)

## Multiple Images

Let's show multiple images in sequence:

![Sample 1](./sample1.jpg)

![Sample 2](./sample2.jpg)

![Sample 3](./sample3.png)

## Image in a List

Here's a bulleted list with images:

- First item
- ![Inline image in list](./sample1.jpg)
- Third item

## Parent Directory Reference

Here's an image from the parent directory:

![Parent directory image](../parent_test.jpg)

## Absolute Path

You can also use absolute paths (though they're less portable):

![Absolute path](/Users/zach/Projects/warp/editor/test_fixtures/images/sample2.jpg)

## Empty Alt Text

![](./sample3.png)

## Text After Images

Here's some regular text after an image. The image should be rendered inline with the text flow.

![Sample](./sample1.jpg)

This text comes after the image.

---

## End of Test

That covers the basic image rendering scenarios!
