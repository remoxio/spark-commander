// sc-gtkstyle: the desktop's GTK 3 theme (Xfce's, MATE's, Cinnamon's) draws
// SparkCommander's controls, each in every state, into images on stdout, as
// sc-qtstyle does with Qt's, and says its colours:
//   sc-gtkstyle <scale> <button height> | sc-gtkstyle colors
// Header lines, "color <name> <rrggbb>" and "size <name> <pixels>" lines,
// then per image "image <name> <width> <height>" and its straight RGBA
// pixels; images in device pixels, sizes in logical ones, "end" last.

#include <gtk/gtk.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static double scale = 1;

// The pixels of `surface` from column `x`, `width` wide (device pixels).
static void put(const char *name, cairo_surface_t *surface, int x, int width) {
    cairo_surface_flush(surface);
    int height = cairo_image_surface_get_height(surface);
    int stride = cairo_image_surface_get_stride(surface);
    const unsigned char *data = cairo_image_surface_get_data(surface);
    printf("image %s %d %d\n", name, width, height);
    fflush(stdout);
    unsigned char *row = g_malloc(width * 4);
    for (int y = 0; y < height; y++) {
        const guint32 *pixels = (const guint32 *)(data + y * stride) + x;
        for (int i = 0; i < width; i++) {
            // Cairo's are premultiplied ARGB in a word.
            guint32 p = pixels[i];
            unsigned a = p >> 24, r = p >> 16 & 255, g = p >> 8 & 255, b = p & 255;
            if (a) {
                r = r * 255 / a, g = g * 255 / a, b = b * 255 / a;
            }
            unsigned char *out = row + i * 4;
            out[0] = r, out[1] = g, out[2] = b, out[3] = a;
        }
        fwrite(row, 1, width * 4, stdout);
    }
    g_free(row);
}

// A CSS node under `parent`, as GTK builds a widget's: so the theme's rules
// for it, and for where it is, apply. `classes` space-separated.
static GtkStyleContext *node(GtkStyleContext *parent, GType type, const char *name, const char *classes) {
    GtkWidgetPath *path = parent ? gtk_widget_path_copy(gtk_style_context_get_path(parent)) : gtk_widget_path_new();
    gtk_widget_path_append_type(path, type);
    gtk_widget_path_iter_set_object_name(path, -1, name);
    gchar **each = g_strsplit(classes ? classes : "", " ", -1);
    for (gchar **klass = each; *klass; klass++) {
        if (**klass) gtk_widget_path_iter_add_class(path, -1, *klass);
    }
    g_strfreev(each);
    GtkStyleContext *context = gtk_style_context_new();
    gtk_style_context_set_path(context, path);
    gtk_style_context_set_parent(context, parent);
    gtk_widget_path_unref(path);
    return context;
}

enum { NORMAL, HOVERED, PRESSED, DISABLED };
static const char *const STATES[] = {"normal", "hovered", "pressed", "disabled"};

static GtkStateFlags flags(int state, gboolean on) {
    GtkStateFlags flags = state == HOVERED  ? GTK_STATE_FLAG_PRELIGHT
                          : state == PRESSED ? GTK_STATE_FLAG_PRELIGHT | GTK_STATE_FLAG_ACTIVE
                          : state == DISABLED ? GTK_STATE_FLAG_INSENSITIVE
                                              : GTK_STATE_FLAG_NORMAL;
    return on ? flags | GTK_STATE_FLAG_CHECKED : flags;
}

// An image `width` × `height` logical pixels at the scale, cleared.
static cairo_surface_t *canvas(int width, int height, cairo_t **cr) {
    cairo_surface_t *surface = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, width * scale + 0.5, height * scale + 0.5);
    cairo_surface_set_device_scale(surface, scale, scale);
    *cr = cairo_create(surface);
    return surface;
}

// `context`'s box drawn in `width` × `height`; a check box's mark too, inside
// its border and padding, as GTK does. Its margins, the space around it, are
// the app's to lay out.
static void draw(GtkStyleContext *context, cairo_t *cr, double width, double height, gboolean check) {
    gtk_render_background(context, cr, 0, 0, width, height);
    gtk_render_frame(context, cr, 0, 0, width, height);
    if (!check) return;
    GtkStateFlags state = gtk_style_context_get_state(context);
    GtkBorder padding, border;
    gtk_style_context_get_padding(context, state, &padding);
    gtk_style_context_get_border(context, state, &border);
    double left = padding.left + border.left, top = padding.top + border.top;
    gtk_render_check(context, cr, left, top, width - left - padding.right - border.right, height - top - padding.bottom - border.bottom);
}

// A node's size: its min-width and min-height with padding and border.
static void size_of(GtkStyleContext *context, int *width, int *height) {
    GtkStateFlags state = gtk_style_context_get_state(context);
    GtkBorder padding, border;
    gtk_style_context_get(context, state, "min-width", width, "min-height", height, NULL);
    gtk_style_context_get_padding(context, state, &padding);
    gtk_style_context_get_border(context, state, &border);
    *width += padding.left + padding.right + border.left + border.right;
    *height += padding.top + padding.bottom + border.top + border.bottom;
}

// `surface`'s span from column `x`, `span` wide (device pixels), as `name`'s
// left and right ends, `end` wide, and a strip of its middle, which the app
// stretches to any width.
static void cut(const char *name, cairo_surface_t *surface, int x, int span, int end) {
    char part[96];
    snprintf(part, sizeof part, "%s.left", name);
    put(part, surface, x, end);
    snprintf(part, sizeof part, "%s.middle", name);
    put(part, surface, x + span / 2 - 2, 4);
    snprintf(part, sizeof part, "%s.right", name);
    put(part, surface, x + span - end, end);
}

// `context`'s box drawn `width` × `height` and its colour at (x, y): themes paint gradients and images.
static GdkRGBA sample(GtkStyleContext *context, int width, int height, int x, int y) {
    cairo_t *cr;
    cairo_surface_t *surface = canvas(width, height, &cr);
    draw(context, cr, width, height, FALSE);
    cairo_destroy(cr);
    cairo_surface_flush(surface);
    int stride = cairo_image_surface_get_stride(surface);
    guint32 p = *(guint32 *)(cairo_image_surface_get_data(surface) + stride * (int)(y * scale) + (int)(x * scale) * 4);
    cairo_surface_destroy(surface);
    double alpha = (p >> 24) / 255.;
    if (alpha == 0) return (GdkRGBA){0, 0, 0, 0};
    return (GdkRGBA){(p >> 16 & 255) / 255. / alpha, (p >> 8 & 255) / 255. / alpha, (p & 255) / 255. / alpha, alpha};
}

// A node's height around a line of text.
static int height_of(GtkStyleContext *context, int text) {
    GtkStateFlags state = gtk_style_context_get_state(context);
    GtkBorder padding, border;
    int min_height;
    gtk_style_context_get(context, state, "min-height", &min_height, NULL);
    gtk_style_context_get_padding(context, state, &padding);
    gtk_style_context_get_border(context, state, &border);
    return border.top + padding.top + MAX(min_height, text) + padding.bottom + border.bottom;
}

// A colour over the window's, as an opaque "rrggbb".
static void put_color(const char *name, GdkRGBA color, GdkRGBA under) {
    double a = color.alpha;
    int r = (color.red * a + under.red * (1 - a)) * 255 + 0.5;
    int g = (color.green * a + under.green * (1 - a)) * 255 + 0.5;
    int b = (color.blue * a + under.blue * (1 - a)) * 255 + 0.5;
    printf("color %s %02x%02x%02x\n", name, r, g, b);
}

// Notebook tabs as GTK lays them out: the bar (`header`) with a tab on it,
// normal, hovered and the open one, each cut into the tab's ends and a strip
// of its middle, its margins with it; a strip of the bar beyond the tab; the
// bar's height and the label's inset ("size" lines) and colours.
static void tabs(GtkStyleContext *window, GdkRGBA background) {
    GtkStyleContext *notebook = node(window, GTK_TYPE_NOTEBOOK, "notebook", NULL);
    GtkStyleContext *header = node(notebook, G_TYPE_NONE, "header", "top");
    GtkStyleContext *strip = node(header, G_TYPE_NONE, "tabs", NULL);
    GtkStyleContext *tab = node(strip, G_TYPE_NONE, "tab", NULL);
    GtkStyleContext *label = node(tab, GTK_TYPE_LABEL, "label", NULL);
    GtkWidget *sample = g_object_ref_sink(gtk_label_new("Tab"));
    int text;
    gtk_widget_get_preferred_height(sample, NULL, &text);
    // The row may reach past the bar's padding and border (Greybird's -1 px: the open tab covers the line).
    GtkBorder header_border, header_padding, row, margin, border, padding;
    gtk_style_context_get_border(header, GTK_STATE_FLAG_NORMAL, &header_border);
    gtk_style_context_get_padding(header, GTK_STATE_FLAG_NORMAL, &header_padding);
    gtk_style_context_get_margin(strip, GTK_STATE_FLAG_NORMAL, &row);
    gtk_style_context_get_margin(tab, GTK_STATE_FLAG_NORMAL, &margin);
    gtk_style_context_get_border(tab, GTK_STATE_FLAG_NORMAL, &border);
    gtk_style_context_get_padding(tab, GTK_STATE_FLAG_NORMAL, &padding);
    int tab_height = height_of(tab, text);
    int top = header_border.top + header_padding.top + row.top;
    int height = top + margin.top + tab_height + margin.bottom + row.bottom + header_padding.bottom + header_border.bottom;
    int left = header_border.left + header_padding.left + row.left, width = tab_height * 3;
    int span = margin.left + width + margin.right, full = left + span + tab_height * 2;
    static const char *const NAMES[] = {"normal", "hovered", "selected"};
    static const GtkStateFlags FLAGS[] = {GTK_STATE_FLAG_NORMAL, GTK_STATE_FLAG_PRELIGHT, GTK_STATE_FLAG_CHECKED};
    for (int i = 0; i < 3; i++) {
        gtk_style_context_set_state(tab, FLAGS[i]);
        gtk_style_context_set_state(label, FLAGS[i]);
        GtkBorder own;
        gtk_style_context_get_margin(tab, FLAGS[i], &own);
        cairo_t *cr;
        cairo_surface_t *surface = canvas(full, height, &cr);
        draw(header, cr, full, height, FALSE);
        cairo_translate(cr, left + own.left, top + own.top);
        draw(tab, cr, width, tab_height, FALSE);
        cairo_destroy(cr);
        // Device pixels: the tab's span, its ends half its height each.
        char name[64];
        snprintf(name, sizeof name, "tab.%s", NAMES[i]);
        cut(name, surface, left * scale + 0.5, span * scale + 0.5, tab_height / 2. * scale + 0.5);
        if (i == 0) put("tabbar", surface, cairo_image_surface_get_width(surface) - (int)(tab_height * scale), 4);
        cairo_surface_destroy(surface);
        GdkRGBA color;
        gtk_style_context_get_color(label, FLAGS[i], &color);
        snprintf(name, sizeof name, "tab.%s", NAMES[i]);
        put_color(name, color, background);
    }
    printf("size tab.height %d\nsize tab.inset %d\n", height, margin.left + border.left + padding.left);
    g_object_unref(sample);
    g_object_unref(label);
    g_object_unref(tab);
    g_object_unref(strip);
    g_object_unref(header);
    g_object_unref(notebook);
}

// Menus as GTK draws them: the menu bar's open title and a menu's highlighted
// row, cut as a tab; the menu's fill, border, padding, and corners under the
// menu bar and elsewhere; the row's height, inset and text.
static void menus(GtkStyleContext *window, GdkRGBA background) {
    GtkStyleContext *bar = node(window, GTK_TYPE_MENU_BAR, "menubar", NULL);
    GtkStyleContext *title = node(bar, GTK_TYPE_MENU_ITEM, "menuitem", NULL);
    GtkStyleContext *title_label = node(title, GTK_TYPE_LABEL, "label", NULL);
    // A menu's window hangs under the item it opened from, as GTK's CSS nodes do.
    GtkStyleContext *popup = node(title, GTK_TYPE_WINDOW, "window", "popup csd");
    GtkStyleContext *decoration = node(popup, G_TYPE_NONE, "decoration", NULL);
    GtkStyleContext *menu = node(popup, GTK_TYPE_MENU, "menu", NULL);
    GtkStyleContext *row = node(menu, GTK_TYPE_MENU_ITEM, "menuitem", NULL);
    GtkStyleContext *row_label = node(row, GTK_TYPE_LABEL, "label", NULL);
    GtkStyleContext *loose = node(NULL, GTK_TYPE_WINDOW, "window", "popup csd");
    GtkStyleContext *loose_decoration = node(loose, G_TYPE_NONE, "decoration", NULL);
    GtkWidget *line = g_object_ref_sink(gtk_label_new("Menu"));
    int text;
    gtk_widget_get_preferred_height(line, NULL, &text);
    GdkRGBA color;
    // The open title and the highlighted row: their nodes hovered.
    struct { const char *name; GtkStyleContext *box, *label; } parts[] = {{"menubar.open", title, title_label}, {"menu.row", row, row_label}};
    for (int i = 0; i < 2; i++) {
        gtk_style_context_set_state(parts[i].box, GTK_STATE_FLAG_PRELIGHT);
        gtk_style_context_set_state(parts[i].label, GTK_STATE_FLAG_PRELIGHT);
        int height = height_of(parts[i].box, text), width = height * 4;
        cairo_t *cr;
        cairo_surface_t *surface = canvas(width, height, &cr);
        draw(parts[i].box, cr, width, height, FALSE);
        cairo_destroy(cr);
        cut(parts[i].name, surface, 0, cairo_image_surface_get_width(surface), height / 2. * scale + 0.5);
        cairo_surface_destroy(surface);
        gtk_style_context_get_color(parts[i].label, GTK_STATE_FLAG_PRELIGHT, &color);
        put_color(parts[i].name, color, background);
        if (i == 1) {
            GtkBorder padding, border;
            gtk_style_context_get_padding(row, GTK_STATE_FLAG_PRELIGHT, &padding);
            gtk_style_context_get_border(row, GTK_STATE_FLAG_PRELIGHT, &border);
            printf("size menu.row.height %d\nsize menu.row.inset %d\n", height, border.left + padding.left);
        }
    }
    // The menu's fill in its middle, its border at its edge.
    GtkBorder border, padding;
    gtk_style_context_get_border(menu, GTK_STATE_FLAG_NORMAL, &border);
    gtk_style_context_get_padding(menu, GTK_STATE_FLAG_NORMAL, &padding);
    GdkRGBA fill = sample(menu, 40, 40, 20, 20);
    put_color("menu", fill, background);
    put_color("menu.border", border.left > 0 ? sample(menu, 40, 40, 0, 20) : fill, background);
    int radius, loose_radius;
    gtk_style_context_get(decoration, GTK_STATE_FLAG_NORMAL, "border-radius", &radius, NULL);
    gtk_style_context_get(loose_decoration, GTK_STATE_FLAG_NORMAL, "border-radius", &loose_radius, NULL);
    printf("size menu.padding %d\nsize menu.radius %d\nsize menu.context.radius %d\n", padding.top, radius, loose_radius);
    g_object_unref(line);
    GtkStyleContext *nodes[] = {loose_decoration, loose, row_label, row, menu, decoration, popup, title_label, title, bar};
    for (size_t i = 0; i < G_N_ELEMENTS(nodes); i++) g_object_unref(nodes[i]);
}

int main(int argc, char **argv) {
    if (!gtk_init_check(&argc, &argv)) {
        fprintf(stderr, "sc-gtkstyle: no display\n");
        return 1;
    }
    gboolean colors_only = argc == 2 && !strcmp(argv[1], "colors");
    if (argc < 3 && !colors_only) {
        fprintf(stderr, "usage: sc-gtkstyle <scale> <button height> | sc-gtkstyle colors\n");
        return 2;
    }
    scale = colors_only ? 1 : g_ascii_strtod(argv[1], NULL);
    int button_height = colors_only ? 24 : atoi(argv[2]);
    if (scale <= 0 || button_height <= 0) return 2;

    // GTK_THEME, when set, wins over the settings' theme.
    gchar *theme = g_strdup(g_getenv("GTK_THEME"));
    if (!theme) g_object_get(gtk_settings_get_default(), "gtk-theme-name", &theme, NULL);
    GtkStyleContext *window = node(NULL, GTK_TYPE_WINDOW, "window", "background");
    GdkRGBA background = {1, 1, 1, 1};
    gtk_style_context_lookup_color(window, "theme_bg_color", &background);
    double lightness = 0.299 * background.red + 0.587 * background.green + 0.114 * background.blue;
    printf("sc-gtkstyle 1\nstyle %s\ndark %d\n", theme ? theme : "", lightness < 0.5);
    g_free(theme);

    // The named colours GTK's apps paint with, where the theme defines them.
    static const char *const COLORS[] = {"theme_bg_color", "theme_fg_color", "theme_base_color", "theme_text_color",
                                         "theme_selected_bg_color", "theme_selected_fg_color", "borders", "error_color"};
    for (size_t i = 0; i < G_N_ELEMENTS(COLORS); i++) {
        GdkRGBA color;
        if (gtk_style_context_lookup_color(window, COLORS[i], &color)) put_color(COLORS[i], color, background);
    }
    // A push button's fill, its middle as drawn.
    GtkStyleContext *plain = node(window, GTK_TYPE_BUTTON, "button", "text-button");
    GdkRGBA fill = sample(plain, button_height, button_height, button_height / 2, button_height / 2);
    if (fill.alpha > 0) put_color("button", fill, background);
    g_object_unref(plain);
    if (colors_only) {
        printf("end\n");
        return 0;
    }

    GtkStyleContext *checkbutton = node(window, GTK_TYPE_CHECK_BUTTON, "checkbutton", NULL);
    GtkStyleContext *check = node(checkbutton, G_TYPE_NONE, "check", NULL);
    int width, height;
    size_of(check, &width, &height);
    for (int on = 0; on <= 1; on++) {
        for (int state = NORMAL; state <= DISABLED; state++) {
            gtk_style_context_set_state(checkbutton, flags(state, on));
            gtk_style_context_set_state(check, flags(state, on));
            cairo_t *cr;
            cairo_surface_t *surface = canvas(width, height, &cr);
            draw(check, cr, width, height, TRUE);
            cairo_destroy(cr);
            char name[64];
            snprintf(name, sizeof name, "checkbox.%s.%s", on ? "on" : "off", STATES[state]);
            put(name, surface, 0, cairo_image_surface_get_width(surface));
            cairo_surface_destroy(surface);
        }
    }

    // A button four times as wide as high, cut into its ends (half its height
    // each) and a strip of its middle, which the app stretches to any width.
    for (int is_default = 0; is_default <= 1; is_default++) {
        GtkStyleContext *button = node(window, GTK_TYPE_BUTTON, "button", "text-button");
        if (is_default) gtk_style_context_add_class(button, GTK_STYLE_CLASS_DEFAULT);
        for (int state = NORMAL; state <= DISABLED; state++) {
            gtk_style_context_set_state(button, flags(state, FALSE));
            cairo_t *cr;
            cairo_surface_t *surface = canvas(button_height * 4, button_height, &cr);
            draw(button, cr, button_height * 4, button_height, FALSE);
            cairo_destroy(cr);
            char name[64];
            snprintf(name, sizeof name, "button.%s.%s", is_default ? "default" : "plain", STATES[state]);
            cut(name, surface, 0, cairo_image_surface_get_width(surface), cairo_image_surface_get_height(surface) / 2);
            cairo_surface_destroy(surface);
        }
        g_object_unref(button);
    }
    tabs(window, background);
    menus(window, background);
    printf("end\n");
    return 0;
}
