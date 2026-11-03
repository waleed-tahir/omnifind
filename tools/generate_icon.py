"""
Generate OmniFind Application Icon
Faithful vector-quality replica of proposed Windows 11 Fluent design:
- Rounded squircle tile container with soft ambient drop shadow and frosted background
- 3 aerodynamic cyan motion blur trails bleeding gracefully across the left boundary
- Seamless back folder (cyan) with curved tab, smooth shoulder ramp, and frosted inner card
- Front folder (royal blue) showing through transparent magnifying glass lens
- Precision magnifying glass with crystal-cyan rim, 45° ergonomic handle, and glass specular reflection
- Multi-resolution Windows .ico (256, 128, 64, 48, 32, 16) and 512x512 .png
"""

import math
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageChops

CANVAS_SIZE = 1024


def create_gradient_vertical(size, top_color, bottom_color):
    """Creates a vertical linear gradient RGBA image."""
    w, h = size
    t_r, t_g, t_b, t_a = top_color
    b_r, b_g, b_b, b_a = bottom_color
    
    y = np.linspace(0, 1, h, dtype=np.float32)[:, None]
    
    r = (t_r * (1 - y) + b_r * y).astype(np.uint8)
    g = (t_g * (1 - y) + b_g * y).astype(np.uint8)
    b = (t_b * (1 - y) + b_b * y).astype(np.uint8)
    a = (t_a * (1 - y) + b_a * y).astype(np.uint8)
    
    r = np.repeat(r, w, axis=1)
    g = np.repeat(g, w, axis=1)
    b = np.repeat(b, w, axis=1)
    a = np.repeat(a, w, axis=1)
    
    rgba = np.stack([r, g, b, a], axis=2)
    return Image.fromarray(rgba, 'RGBA')


def create_gradient_diagonal(size, top_left_color, bottom_right_color):
    """Creates a 45-degree diagonal linear gradient RGBA image."""
    w, h = size
    tl_r, tl_g, tl_b, tl_a = top_left_color
    br_r, br_g, br_b, br_a = bottom_right_color
    
    x = np.linspace(0, 1, w, dtype=np.float32)[None, :]
    y = np.linspace(0, 1, h, dtype=np.float32)[:, None]
    diag = (x + y) / 2.0
    diag = np.clip(diag, 0.0, 1.0)
    
    r = (tl_r * (1 - diag) + br_r * diag).astype(np.uint8)
    g = (tl_g * (1 - diag) + br_g * diag).astype(np.uint8)
    b = (tl_b * (1 - diag) + br_b * diag).astype(np.uint8)
    a = (tl_a * (1 - diag) + br_a * diag).astype(np.uint8)
    
    rgba = np.stack([r, g, b, a], axis=2)
    return Image.fromarray(rgba, 'RGBA')


def get_stroke_mask(solid_mask, stroke_width):
    """Computes exact morphological stroke mask of any solid mask without internal seams."""
    filter_size = stroke_width if stroke_width % 2 == 1 else stroke_width + 1
    eroded = solid_mask.filter(ImageFilter.MinFilter(size=filter_size))
    return ImageChops.subtract(solid_mask, eroded)


def overlay_color_with_mask(canvas, color_rgba, mask):
    """Alpha-composites a solid color onto canvas modulated by grayscale mask."""
    r, g, b, a = color_rgba
    alpha = (np.array(mask, dtype=np.uint16) * a // 255).astype(np.uint8)
    h, w = alpha.shape
    r_arr = np.full((h, w), r, dtype=np.uint8)
    g_arr = np.full((h, w), g, dtype=np.uint8)
    b_arr = np.full((h, w), b, dtype=np.uint8)
    layer = Image.fromarray(np.stack([r_arr, g_arr, b_arr, alpha], axis=2), 'RGBA')
    return Image.alpha_composite(canvas, layer)


def overlay_image_with_mask(canvas, img_rgba, mask):
    """Alpha-composites an RGBA image onto canvas modulated by grayscale mask."""
    r, g, b, a = img_rgba.split()
    a_arr = (np.array(a, dtype=np.uint16) * np.array(mask, dtype=np.uint16) // 255).astype(np.uint8)
    layer = Image.merge('RGBA', (r, g, b, Image.fromarray(a_arr, 'L')))
    return Image.alpha_composite(canvas, layer)


def render_icon():
    canvas = Image.new('RGBA', (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
    
    # =========================================================================
    # 1. SQUIRCLE TILE CONTAINER
    # =========================================================================
    tile_box = [64, 52, 960, 948]
    tile_radius = 210
    
    # Ambient Drop Shadow beneath squircle
    shadow_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    shadow_draw = ImageDraw.Draw(shadow_mask)
    shadow_box = [tile_box[0], tile_box[1] + 34, tile_box[2], tile_box[3] + 34]
    shadow_draw.rounded_rectangle(shadow_box, radius=tile_radius, fill=255)
    shadow_mask = shadow_mask.filter(ImageFilter.GaussianBlur(34))
    canvas = overlay_color_with_mask(canvas, (15, 35, 75, 75), shadow_mask)
    
    # Squircle Tile Background
    tile_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    tile_draw = ImageDraw.Draw(tile_mask)
    tile_draw.rounded_rectangle(tile_box, radius=tile_radius, fill=255)
    
    tile_gradient = create_gradient_vertical(
        (CANVAS_SIZE, CANVAS_SIZE),
        (255, 255, 255, 255),
        (228, 238, 248, 255)
    )
    canvas = overlay_image_with_mask(canvas, tile_gradient, tile_mask)
    
    # Squircle Border Stroke
    border_stroke_mask = get_stroke_mask(tile_mask, stroke_width=5)
    border_gradient = create_gradient_vertical(
        (CANVAS_SIZE, CANVAS_SIZE),
        (255, 255, 255, 250),
        (185, 210, 235, 180)
    )
    canvas = overlay_image_with_mask(canvas, border_gradient, border_stroke_mask)
    
    # =========================================================================
    # 2. SPEED TRAILS (MOTION BLUR BLEEDING PAST THE LEFT EDGE)
    # =========================================================================
    speed_layer = Image.new('RGBA', (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
    speed_draw = ImageDraw.Draw(speed_layer)
    
    streaks = [
        (260, 48, -40, 310, 160),
        (395, 56, -60, 330, 190),
        (530, 48, -30, 290, 150),
    ]
    
    for y_c, h, x0, x1, alpha in streaks:
        y0 = y_c - h // 2
        y1 = y_c + h // 2
        
        streak_img = Image.new('RGBA', (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
        s_draw = ImageDraw.Draw(streak_img)
        s_draw.rounded_rectangle([x0, y0, x1, y1], radius=h//2, fill=(80, 205, 255, alpha))
        
        streak_blurred = streak_img.filter(ImageFilter.BoxBlur((48, 8)))
        speed_layer = Image.alpha_composite(speed_layer, streak_blurred)
    
    canvas = Image.alpha_composite(canvas, speed_layer)
    
    # =========================================================================
    # 3. BACK FOLDER (UNIFIED CYAN SILHOUETTE WITH INNER FROSTED CARD)
    # =========================================================================
    back_folder_solid = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    bf_draw = ImageDraw.Draw(back_folder_solid)
    
    # Main folder body
    bf_draw.rounded_rectangle([175, 238, 660, 690], radius=56, fill=255)
    # Tab
    bf_draw.rounded_rectangle([175, 178, 410, 260], radius=44, corners=(True, False, False, False), fill=255)
    # Tab ramp
    ramp_points = [
        (380, 178),
        (420, 178),
        (480, 238),
        (380, 238),
    ]
    bf_draw.polygon(ramp_points, fill=255)
    bf_draw.ellipse([400, 178, 440, 218], fill=255)
    
    # Back Folder Drop Shadow
    bf_shadow_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    bf_s_draw = ImageDraw.Draw(bf_shadow_mask)
    bf_s_draw.rounded_rectangle([175, 238 + 16, 660, 690 + 16], radius=56, fill=255)
    bf_s_draw.rounded_rectangle([175, 178 + 16, 410, 260 + 16], radius=44, corners=(True, False, False, False), fill=255)
    bf_s_draw.polygon([(x, y + 16) for x, y in ramp_points], fill=255)
    bf_shadow_mask = bf_shadow_mask.filter(ImageFilter.GaussianBlur(24))
    canvas = overlay_color_with_mask(canvas, (20, 70, 130, 65), bf_shadow_mask)
    
    # Back Folder Gradient
    bf_gradient = create_gradient_diagonal(
        (CANVAS_SIZE, CANVAS_SIZE),
        (76, 196, 238, 255),
        (24, 142, 222, 255)
    )
    canvas = overlay_image_with_mask(canvas, bf_gradient, back_folder_solid)
    
    # Unified Back Folder Outline
    bf_outline_mask = get_stroke_mask(back_folder_solid, stroke_width=7)
    canvas = overlay_color_with_mask(canvas, (16, 112, 185, 255), bf_outline_mask)
    
    # Inner Frosted Card / Document inside Back Folder
    doc_box = [242, 278, 612, 630]
    doc_radius = 42
    
    doc_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    doc_draw = ImageDraw.Draw(doc_mask)
    doc_draw.rounded_rectangle(doc_box, radius=doc_radius, fill=255)
    
    doc_gradient = create_gradient_vertical(
        (CANVAS_SIZE, CANVAS_SIZE),
        (255, 255, 255, 90),
        (205, 242, 255, 45)
    )
    canvas = overlay_image_with_mask(canvas, doc_gradient, doc_mask)
    
    doc_border_mask = get_stroke_mask(doc_mask, stroke_width=6)
    canvas = overlay_color_with_mask(canvas, (255, 255, 255, 195), doc_border_mask)
    
    # =========================================================================
    # 4. FRONT FOLDER (ROYAL BLUE TILE WITH PILL ACCENT)
    # =========================================================================
    front_box = [288, 332, 768, 812]
    front_radius = 72
    
    # Front Folder Shadow
    ff_shadow_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    ff_s_draw = ImageDraw.Draw(ff_shadow_mask)
    ff_shadow_box = [front_box[0] - 8, front_box[1] + 16, front_box[2] + 8, front_box[3] + 22]
    ff_s_draw.rounded_rectangle(ff_shadow_box, radius=front_radius, fill=255)
    ff_shadow_mask = ff_shadow_mask.filter(ImageFilter.GaussianBlur(24))
    canvas = overlay_color_with_mask(canvas, (10, 48, 105, 115), ff_shadow_mask)
    
    # Front Folder Solid Mask & Gradient
    front_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    ff_draw = ImageDraw.Draw(front_mask)
    ff_draw.rounded_rectangle(front_box, radius=front_radius, fill=255)
    
    front_gradient = create_gradient_vertical(
        (CANVAS_SIZE, CANVAS_SIZE),
        (36, 150, 238, 255),
        (12, 94, 204, 255)
    )
    canvas = overlay_image_with_mask(canvas, front_gradient, front_mask)
    
    # Front Folder Dark Rim
    front_rim_mask = get_stroke_mask(front_mask, stroke_width=7)
    canvas = overlay_color_with_mask(canvas, (12, 78, 168, 255), front_rim_mask)
    
    # Top Highlight along front folder edge
    th_clip = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    th_c_draw = ImageDraw.Draw(th_clip)
    th_c_draw.rectangle([front_box[0], front_box[1], front_box[2], front_box[1] + 45], fill=255)
    
    th_mask = ImageChops.multiply(front_rim_mask, th_clip)
    canvas = overlay_color_with_mask(canvas, (255, 255, 255, 95), th_mask)
    
    # Pill / Capsule Accent on top-right of front folder
    pill_box = [532, 400, 692, 448]
    pill_radius = 24
    
    pill_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    p_draw = ImageDraw.Draw(pill_mask)
    p_draw.rounded_rectangle(pill_box, radius=pill_radius, fill=255)
    
    pill_gradient = create_gradient_vertical(
        (CANVAS_SIZE, CANVAS_SIZE),
        (160, 218, 252, 175),
        (130, 195, 245, 140)
    )
    canvas = overlay_image_with_mask(canvas, pill_gradient, pill_mask)
    
    pill_border_mask = get_stroke_mask(pill_mask, stroke_width=4)
    canvas = overlay_color_with_mask(canvas, (240, 250, 255, 220), pill_border_mask)
    
    # =========================================================================
    # 5. MAGNIFYING GLASS (SEARCH LENS & HANDLE)
    # =========================================================================
    lens_cx = 636
    lens_cy = 644
    r_outer = 136
    r_inner = 88
    
    # Handle geometry: 45 degrees down-right
    rad = math.radians(45)
    cos45 = math.cos(rad)
    sin45 = math.sin(rad)
    
    h_start_dist = r_outer - 6
    h_end_dist = r_outer + 120
    h_width = 50
    
    perp_x = -sin45 * (h_width / 2.0)
    perp_y = cos45 * (h_width / 2.0)
    
    p1 = (lens_cx + h_start_dist * cos45 - perp_x, lens_cy + h_start_dist * sin45 - perp_y)
    p2 = (lens_cx + h_start_dist * cos45 + perp_x, lens_cy + h_start_dist * sin45 + perp_y)
    p3 = (lens_cx + h_end_dist * cos45 + perp_x, lens_cy + h_end_dist * sin45 + perp_y)
    p4 = (lens_cx + h_end_dist * cos45 - perp_x, lens_cy + h_end_dist * sin45 - perp_y)
    
    cap_cx = lens_cx + h_end_dist * cos45
    cap_cy = lens_cy + h_end_dist * sin45
    
    # Build unified magnifying glass mask
    mg_solid = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    mg_s_draw = ImageDraw.Draw(mg_solid)
    
    mg_s_draw.ellipse([lens_cx - r_outer, lens_cy - r_outer, lens_cx + r_outer, lens_cy + r_outer], fill=255)
    mg_s_draw.polygon([p1, p2, p3, p4], fill=255)
    mg_s_draw.ellipse([cap_cx - h_width/2, cap_cy - h_width/2, cap_cx + h_width/2, cap_cy + h_width/2], fill=255)
    mg_s_draw.ellipse([lens_cx - r_inner, lens_cy - r_inner, lens_cx + r_inner, lens_cy + r_inner], fill=0)
    
    # Magnifying Glass Shadow
    mg_shadow_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    mg_sh_draw = ImageDraw.Draw(mg_shadow_mask)
    off_x, off_y = 12, 16
    mg_sh_draw.ellipse([lens_cx - r_outer + off_x, lens_cy - r_outer + off_y, lens_cx + r_outer + off_x, lens_cy + r_outer + off_y], fill=255)
    mg_sh_draw.polygon([(x + off_x, y + off_y) for x, y in [p1, p2, p3, p4]], fill=255)
    mg_sh_draw.ellipse([cap_cx - h_width/2 + off_x, cap_cy - h_width/2 + off_y, cap_cx + h_width/2 + off_x, cap_cy + h_width/2 + off_y], fill=255)
    mg_sh_draw.ellipse([lens_cx - r_inner + off_x, lens_cy - r_inner + off_y, lens_cx + r_inner + off_x, lens_cy + r_inner + off_y], fill=0)
    
    mg_shadow_mask = mg_shadow_mask.filter(ImageFilter.GaussianBlur(16))
    canvas = overlay_color_with_mask(canvas, (8, 36, 80, 130), mg_shadow_mask)
    
    # Magnifying Glass Body Fill (Rim & Handle Gradient)
    mg_gradient = create_gradient_diagonal(
        (CANVAS_SIZE, CANVAS_SIZE),
        (234, 248, 255, 255),
        (130, 190, 248, 255)
    )
    canvas = overlay_image_with_mask(canvas, mg_gradient, mg_solid)
    
    # Lens Glass Interior (Subtle bright glass glow + specular reflection arc)
    glass_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    g_draw = ImageDraw.Draw(glass_mask)
    g_draw.ellipse([lens_cx - r_inner, lens_cy - r_inner, lens_cx + r_inner, lens_cy + r_inner], fill=255)
    canvas = overlay_color_with_mask(canvas, (255, 255, 255, 38), glass_mask)
    
    # Specular curved highlight on upper-left of lens glass
    spec_mask = Image.new('L', (CANVAS_SIZE, CANVAS_SIZE), 0)
    sp_draw = ImageDraw.Draw(spec_mask)
    sp_box = [lens_cx - r_inner + 12, lens_cy - r_inner + 12, lens_cx + r_inner - 12, lens_cy + r_inner - 12]
    sp_draw.arc(sp_box, start=170, end=290, fill=255, width=12)
    spec_mask = spec_mask.filter(ImageFilter.GaussianBlur(3))
    canvas = overlay_color_with_mask(canvas, (255, 255, 255, 185), spec_mask)
    
    # Unified Magnifying Glass Stroke
    mg_stroke_mask = get_stroke_mask(mg_solid, stroke_width=6)
    canvas = overlay_color_with_mask(canvas, (14, 72, 152, 255), mg_stroke_mask)
    
    return canvas


from pathlib import Path

def main():
    print("Rendering vector-grade 1024x1024 OmniFind icon...")
    icon_1024 = render_icon()
    
    script_dir = Path(__file__).resolve().parent
    project_dir = script_dir.parent
    
    icon_1024.save(script_dir / "icon_1024.png", "PNG")
    print(f"Saved 1024x1024 preview to {script_dir / 'icon_1024.png'}")
    
    icon_512 = icon_1024.resize((512, 512), Image.Resampling.LANCZOS)
    icon_512.save(project_dir / "src" / "icon.png", "PNG")
    print(f"Saved 512x512 icon to {project_dir / 'src' / 'icon.png'}")
    
    icon_256 = icon_1024.resize((256, 256), Image.Resampling.LANCZOS)
    icon_sizes = [(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)]
    icon_256.save(project_dir / "icon.ico", format="ICO", sizes=icon_sizes)
    print(f"Saved multi-size Windows icon to {project_dir / 'icon.ico'}")


if __name__ == "__main__":
    main()
