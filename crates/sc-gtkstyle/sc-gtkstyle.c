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

// What draws a mark in a box: gtk_render_check, gtk_render_option.
typedef void (*Mark)(GtkStyleContext *, cairo_t *, gdouble, gdouble, gdouble, gdouble);

// `context`'s box drawn in `width` × `height`; a check box's or radio
// button's mark too, inside its border and padding, as GTK does. Its margins,
// the space around it, are the app's to lay out.
static void draw(GtkStyleContext *context, cairo_t *cr, double width, double height, Mark mark) {
    gtk_render_background(context, cr, 0, 0, width, height);
    gtk_render_frame(context, cr, 0, 0, width, height);
    if (!mark) return;
    GtkStateFlags state = gtk_style_context_get_state(context);
    GtkBorder padding, border;
    gtk_style_context_get_padding(context, state, &padding);
    gtk_style_context_get_border(context, state, &border);
    double left = padding.left + border.left, top = padding.top + border.top;
    mark(context, cr, left, top, width - left - padding.right - border.right, height - top - padding.bottom - border.bottom);
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
    draw(context, cr, width, height, NULL);
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
        draw(header, cr, full, height, NULL);
        cairo_translate(cr, left + own.left, top + own.top);
        draw(tab, cr, width, tab_height, NULL);
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
        draw(parts[i].box, cr, width, height, NULL);
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

// As `node`, its parent's only child: :first-child, :last-child and :only-child match.
static GtkStyleContext *alone(GtkStyleContext *parent, GType type, const char *name, const char *classes) {
    GtkStyleContext *own = node(NULL, type, name, classes);
    GtkWidgetPath *path = gtk_widget_path_copy(gtk_style_context_get_path(parent));
    gtk_widget_path_append_with_siblings(path, (GtkWidgetPath *)gtk_style_context_get_path(own), 0);
    GtkStyleContext *context = gtk_style_context_new();
    gtk_style_context_set_path(context, path);
    gtk_style_context_set_parent(context, parent);
    gtk_widget_path_unref(path);
    g_object_unref(own);
    return context;
}

// A whole drawing as `name`.
static void put_whole(const char *name, cairo_surface_t *surface) {
    put(name, surface, 0, cairo_image_surface_get_width(surface));
}

// A node's border and padding on each side.
static GtkBorder edges(GtkStyleContext *context) {
    GtkBorder border, padding;
    GtkStateFlags state = gtk_style_context_get_state(context);
    gtk_style_context_get_border(context, state, &border);
    gtk_style_context_get_padding(context, state, &padding);
    return (GtkBorder){border.left + padding.left, border.right + padding.right, border.top + padding.top, border.bottom + padding.bottom};
}

// The left (or top) of a node's border and padding.
static int inset_of(GtkStyleContext *context, gboolean top) {
    GtkBorder edge = edges(context);
    return top ? edge.top : edge.left;
}

// Check boxes and radio buttons: the box or circle and its mark, unchecked
// and checked, in every state.
static void indicator(GtkStyleContext *window, GType type, const char *widget, const char *box, const char *name, Mark mark) {
    GtkStyleContext *button = node(window, type, widget, NULL);
    GtkStyleContext *context = node(button, G_TYPE_NONE, box, NULL);
    int width, height;
    size_of(context, &width, &height);
    for (int on = 0; on <= 1; on++) {
        for (int state = NORMAL; state <= DISABLED; state++) {
            gtk_style_context_set_state(button, flags(state, on));
            gtk_style_context_set_state(context, flags(state, on));
            cairo_t *cr;
            cairo_surface_t *surface = canvas(width, height, &cr);
            draw(context, cr, width, height, mark);
            cairo_destroy(cr);
            char part[64];
            snprintf(part, sizeof part, "%s.%s.%s", name, on ? "on" : "off", STATES[state]);
            put_whole(part, surface);
            cairo_surface_destroy(surface);
        }
    }
    g_object_unref(context);
    g_object_unref(button);
}

// Text fields: GTK's entry `height` high and four times as wide, normal,
// focused and disabled, whole (the app keeps its corners and stretches the
// rest to any size); its text's inset.
static void fields(GtkStyleContext *window, int height) {
    static const char *const NAMES[] = {"normal", "focused", "disabled"};
    static const GtkStateFlags FLAGS[] = {GTK_STATE_FLAG_NORMAL, GTK_STATE_FLAG_FOCUSED, GTK_STATE_FLAG_INSENSITIVE};
    GtkStyleContext *entry = node(window, GTK_TYPE_ENTRY, "entry", NULL);
    for (int i = 0; i < 3; i++) {
        gtk_style_context_set_state(entry, FLAGS[i]);
        cairo_t *cr;
        cairo_surface_t *surface = canvas(height * 4, height, &cr);
        draw(entry, cr, height * 4, height, NULL);
        cairo_destroy(cr);
        char name[64];
        snprintf(name, sizeof name, "field.%s", NAMES[i]);
        put_whole(name, surface);
        cairo_surface_destroy(surface);
    }
    gtk_style_context_set_state(entry, GTK_STATE_FLAG_NORMAL);
    printf("size field.inset %d\n", inset_of(entry, FALSE));
    g_object_unref(entry);
}

// Combo boxes: GTK's button with its arrow, `height` high and four times as
// wide, in every state, whole; the label's inset and colours, the room the
// arrow takes at the right.
static void combos(GtkStyleContext *window, GdkRGBA background, int height) {
    GtkStyleContext *combobox = node(window, GTK_TYPE_COMBO_BOX, "combobox", NULL);
    GtkStyleContext *linked = node(combobox, GTK_TYPE_BOX, "box", "linked horizontal");
    GtkStyleContext *button = alone(linked, GTK_TYPE_TOGGLE_BUTTON, "button", "combo");
    GtkStyleContext *inner = node(button, GTK_TYPE_BOX, "box", "horizontal");
    GtkStyleContext *label = node(inner, GTK_TYPE_CELL_VIEW, "cellview", NULL);
    GtkStyleContext *arrow = node(inner, G_TYPE_NONE, "arrow", NULL);
    GtkBorder border, padding;
    gtk_style_context_get_border(button, GTK_STATE_FLAG_NORMAL, &border);
    gtk_style_context_get_padding(button, GTK_STATE_FLAG_NORMAL, &padding);
    int arrow_width, arrow_height;
    size_of(arrow, &arrow_width, &arrow_height);
    int size = MAX(MAX(arrow_width, arrow_height), 8), width = height * 4, right = border.right + padding.right + size;
    for (int state = NORMAL; state <= DISABLED; state++) {
        GtkStateFlags state_flags = flags(state, FALSE);
        GtkStyleContext *nodes[] = {button, inner, label, arrow};
        for (size_t i = 0; i < G_N_ELEMENTS(nodes); i++) gtk_style_context_set_state(nodes[i], state_flags);
        cairo_t *cr;
        cairo_surface_t *surface = canvas(width, height, &cr);
        draw(button, cr, width, height, NULL);
        gtk_render_arrow(arrow, cr, G_PI, width - right, (height - size) / 2., size);
        cairo_destroy(cr);
        char name[64];
        snprintf(name, sizeof name, "combo.%s", STATES[state]);
        put_whole(name, surface);
        cairo_surface_destroy(surface);
        GdkRGBA color;
        gtk_style_context_get_color(label, state_flags, &color);
        put_color(name, color, background);
    }
    printf("size combo.inset %d\nsize combo.arrow %d\n", border.left + padding.left, right);
    GtkStyleContext *nodes[] = {arrow, label, inner, button, linked, combobox};
    for (size_t i = 0; i < G_N_ELEMENTS(nodes); i++) g_object_unref(nodes[i]);
}

// A tab's close button as GTK's apps put one on a notebook tab (Xfce
// Terminal's, Thunar's): a flat button with the icon theme's close icon, its
// colour the button's, in every state; MATE's apps' (Caja's, MATE Terminal's)
// the full-colour one, Yaru-MATE's red circle.
static void tab_close(GtkStyleContext *window) {
    GtkStyleContext *notebook = node(window, GTK_TYPE_NOTEBOOK, "notebook", NULL);
    GtkStyleContext *header = node(notebook, G_TYPE_NONE, "header", "top");
    GtkStyleContext *strip = node(header, G_TYPE_NONE, "tabs", NULL);
    GtkStyleContext *tab = node(strip, G_TYPE_NONE, "tab", NULL);
    GtkStyleContext *button = node(tab, GTK_TYPE_BUTTON, "button", "flat image-button");
    GtkStyleContext *image = node(button, GTK_TYPE_IMAGE, "image", NULL);
    int icon = 16, min_width, min_height, icon_scale = (int)scale + (scale > (int)scale);
    gtk_style_context_get(button, GTK_STATE_FLAG_NORMAL, "min-width", &min_width, "min-height", &min_height, NULL);
    GtkBorder edge = edges(button);
    int width = edge.left + edge.right + MAX(min_width, icon), height = edge.top + edge.bottom + MAX(min_height, icon);
    const char *desktop = g_getenv("XDG_CURRENT_DESKTOP");
    const char *close = desktop && strstr(desktop, "MATE") ? "window-close" : "window-close-symbolic";
    GtkIconInfo *info = gtk_icon_theme_lookup_icon_for_scale(gtk_icon_theme_get_default(), close, icon, icon_scale, GTK_ICON_LOOKUP_FORCE_SIZE);
    for (int state = NORMAL; state <= DISABLED; state++) {
        gtk_style_context_set_state(button, flags(state, FALSE));
        gtk_style_context_set_state(image, flags(state, FALSE));
        cairo_t *cr;
        cairo_surface_t *surface = canvas(width, height, &cr);
        draw(button, cr, width, height, NULL);
        GdkPixbuf *pixbuf = info ? gtk_icon_info_load_symbolic_for_context(info, image, NULL, NULL) : NULL;
        if (pixbuf) {
            cairo_surface_t *picture = gdk_cairo_surface_create_from_pixbuf(pixbuf, icon_scale, NULL);
            cairo_set_source_surface(cr, picture, (width - icon) / 2., (height - icon) / 2.);
            cairo_paint(cr);
            cairo_surface_destroy(picture);
            g_object_unref(pixbuf);
        }
        cairo_destroy(cr);
        char name[64];
        snprintf(name, sizeof name, "tabclose.%s", STATES[state]);
        put_whole(name, surface);
        cairo_surface_destroy(surface);
    }
    if (info) g_object_unref(info);
    GtkStyleContext *nodes[] = {image, button, tab, strip, header, notebook};
    for (size_t i = 0; i < G_N_ELEMENTS(nodes); i++) g_object_unref(nodes[i]);
}

// Left plus right of a node's margin, border and padding.
static int across(GtkStyleContext *context, gboolean margin) {
    GtkStateFlags state = gtk_style_context_get_state(context);
    GtkBorder border, padding, space;
    gtk_style_context_get_border(context, state, &border);
    gtk_style_context_get_padding(context, state, &padding);
    gtk_style_context_get_margin(context, state, &space);
    return border.left + border.right + padding.left + padding.right + (margin ? space.left + space.right : 0);
}

// A pixel of an image surface, straight.
static GdkRGBA pixel(cairo_surface_t *surface, int x, int y) {
    guint32 p = *(guint32 *)(cairo_image_surface_get_data(surface) + cairo_image_surface_get_stride(surface) * y + x * 4);
    double alpha = (p >> 24) / 255.;
    if (alpha == 0) return (GdkRGBA){0, 0, 0, 0};
    return (GdkRGBA){(p >> 16 & 255) / 255. / alpha, (p >> 8 & 255) / 255. / alpha, (p & 255) / 255. / alpha, alpha};
}

// Scrollbars as GTK draws them beside a view (not over it, as the app has
// them): a vertical one drawn and measured, its width, the slider's breadth,
// its inset from the outer edge and its corners; the trough's fill and the
// line on the view's side; the slider's fill normal, hovered and dragged, its outline.
static void scrollbars(GtkStyleContext *window, GdkRGBA background) {
    GtkStyleContext *bar = node(window, GTK_TYPE_SCROLLBAR, "scrollbar", "vertical right");
    GtkStyleContext *contents = node(bar, G_TYPE_NONE, "contents", NULL);
    GtkStyleContext *trough = node(contents, G_TYPE_NONE, "trough", NULL);
    GtkStyleContext *slider = node(trough, G_TYPE_NONE, "slider", NULL);
    int slider_width, slider_height;
    size_of(slider, &slider_width, &slider_height);
    GtkBorder margin;
    gtk_style_context_get_margin(slider, GTK_STATE_FLAG_NORMAL, &margin);
    int width = across(bar, TRUE) + across(contents, TRUE) + across(trough, TRUE) + margin.left + margin.right + slider_width, length = 200;
    static const char *const NAMES[] = {"scrollbar.thumb", "scrollbar.thumb.hovered", "scrollbar.thumb.pressed"};
    static const GtkStateFlags FLAGS[] = {GTK_STATE_FLAG_NORMAL, GTK_STATE_FLAG_PRELIGHT, GTK_STATE_FLAG_PRELIGHT | GTK_STATE_FLAG_ACTIVE};
    for (int i = 0; i < 3; i++) {
        gtk_style_context_set_state(slider, FLAGS[i]);
        cairo_t *cr;
        cairo_surface_t *surface = canvas(width, length, &cr);
        // Each node in its parent's content box, inside its own margins.
        GtkStyleContext *nodes[] = {bar, contents, trough};
        double x = 0, y = 0, w = width, h = length;
        for (size_t n = 0; n < G_N_ELEMENTS(nodes); n++) {
            GtkBorder space;
            gtk_style_context_get_margin(nodes[n], GTK_STATE_FLAG_NORMAL, &space);
            x += space.left, y += space.top, w -= space.left + space.right, h -= space.top + space.bottom;
            gtk_render_background(nodes[n], cr, x, y, w, h);
            gtk_render_frame(nodes[n], cr, x, y, w, h);
            GtkBorder edge = edges(nodes[n]);
            x += edge.left, y += edge.top, w -= edge.left + edge.right, h -= edge.top + edge.bottom;
        }
        gtk_render_background(slider, cr, x + margin.left, length / 4, w - margin.left - margin.right, length / 2);
        gtk_render_frame(slider, cr, x + margin.left, length / 4, w - margin.left - margin.right, length / 2);
        cairo_destroy(cr);
        cairo_surface_flush(surface);
        // The slider's columns: where its middle row differs from the trough's beyond it.
        int columns = cairo_image_surface_get_width(surface), middle = length / 2 * scale, beyond = (length - 8) * scale;
        int first = -1, last = -1;
        for (int column = 0; column < columns; column++) {
            GdkRGBA on = pixel(surface, column, middle), off = pixel(surface, column, beyond);
            if (!gdk_rgba_equal(&on, &off)) {
                if (first < 0) first = column;
                last = column;
            }
        }
        GdkRGBA thumb = first < 0 ? background : pixel(surface, (first + last) / 2, middle), rim = first < 0 ? thumb : pixel(surface, first, middle);
        if (first >= 0) put_color(NAMES[i], thumb, background);
        // An outline: its edge unlike its middle, opaque (not the antialiased end of a fill).
        if (i == 0 && first >= 0 && rim.alpha > 0.9 && !gdk_rgba_equal(&rim, &thumb)) put_color("scrollbar.thumb.outline", rim, background);
        if (i == 0) {
            GdkRGBA fill = pixel(surface, columns / 2, beyond), edge = pixel(surface, 0, beyond);
            put_color("scrollbar", fill, background);
            if (!gdk_rgba_equal(&fill, &edge)) put_color("scrollbar.line", edge, background);
            int radius;
            gtk_style_context_get(slider, GTK_STATE_FLAG_NORMAL, "border-radius", &radius, NULL);
            if (first >= 0)
                printf("size scrollbar.width %d\nsize scrollbar.thumb %g\nsize scrollbar.inset %g\nsize scrollbar.radius %d\n", width,
                       (last - first + 1) / scale, (columns - 1 - last) / scale, radius);
        }
        cairo_surface_destroy(surface);
    }
    GtkStyleContext *nodes[] = {slider, trough, contents, bar};
    for (size_t i = 0; i < G_N_ELEMENTS(nodes); i++) g_object_unref(nodes[i]);
}

// A toolbar's toggle button checked (GtkToggleToolButton's flat button), drawn whole, and its corners' size; none where the theme draws nothing.
static void toggle(GtkStyleContext *window, int height) {
    GtkStyleContext *bar = node(window, GTK_TYPE_TOOLBAR, "toolbar", "horizontal");
    GtkStyleContext *item = node(bar, GTK_TYPE_TOGGLE_TOOL_BUTTON, "toolbutton", NULL);
    GtkStyleContext *button = node(item, GTK_TYPE_TOGGLE_BUTTON, "button", "flat toggle");
    gtk_style_context_set_state(button, GTK_STATE_FLAG_CHECKED);
    cairo_t *cr;
    cairo_surface_t *surface = canvas(height, height, &cr);
    draw(button, cr, height, height, NULL);
    cairo_destroy(cr);
    cairo_surface_flush(surface);
    int w = cairo_image_surface_get_width(surface), h = cairo_image_surface_get_height(surface);
    if (pixel(surface, w / 2, h / 2).alpha > 0 || pixel(surface, 1, h / 2).alpha > 0) {
        put_whole("toolbutton.checked", surface);
        int radius;
        GtkBorder border;
        gtk_style_context_get(button, GTK_STATE_FLAG_CHECKED, GTK_STYLE_PROPERTY_BORDER_RADIUS, &radius, NULL);
        gtk_style_context_get_border(button, GTK_STATE_FLAG_CHECKED, &border);
        printf("size toolbutton.corner %d\n", radius + MAX(border.top, border.left) + 1);
    }
    cairo_surface_destroy(surface);
    g_object_unref(button);
    g_object_unref(item);
    g_object_unref(bar);
}

// `watch`: a line "changed" a moment after the desktop's look changes, however
// many settings one change touches; till stdin closes (the app gone).
static guint pending;

static gboolean report(gpointer data) {
    (void)data;
    pending = 0;
    puts("changed");
    fflush(stdout);
    return G_SOURCE_REMOVE;
}

static void changed(void) {
    if (pending) g_source_remove(pending);
    pending = g_timeout_add(300, report, NULL);
}

static void on_notify(GObject *object, GParamSpec *spec, gpointer data) {
    (void)object, (void)spec, (void)data;
    changed();
}

static void on_setting(GSettings *settings, const char *key, gpointer data) {
    (void)settings, (void)key, (void)data;
    changed();
}

static void on_xfconf(GDBusConnection *bus, const char *sender, const char *path, const char *interface,
                      const char *signal, GVariant *parameters, gpointer data) {
    (void)bus, (void)sender, (void)path, (void)interface, (void)signal, (void)parameters, (void)data;
    changed();
}

static gboolean gone(GIOChannel *input, GIOCondition condition, gpointer data) {
    (void)input, (void)condition, (void)data;
    gtk_main_quit();
    return G_SOURCE_REMOVE;
}

// GTK's own settings (theme, icons, font, dark: Xfce's, MATE's and Cinnamon's
// XSETTINGS, GNOME's on Wayland), GNOME's, MATE's and Cinnamon's interface
// settings (accent, colour scheme, fixed-width font), Xfce's xsettings channel.
static void watch(void) {
    static const char *const PROPERTIES[] = {"gtk-theme-name", "gtk-icon-theme-name", "gtk-font-name",
                                             "gtk-application-prefer-dark-theme"};
    for (size_t i = 0; i < G_N_ELEMENTS(PROPERTIES); i++) {
        char signal[64];
        snprintf(signal, sizeof signal, "notify::%s", PROPERTIES[i]);
        g_signal_connect(gtk_settings_get_default(), signal, G_CALLBACK(on_notify), NULL);
    }
    static const char *const SCHEMAS[] = {"org.gnome.desktop.interface", "org.mate.interface", "org.cinnamon.desktop.interface"};
    static const char *const KEYS[] = {"gtk-theme", "icon-theme", "font-name", "monospace-font-name", "color-scheme",
                                       "accent-color", "text-scaling-factor"};
    GSettingsSchemaSource *source = g_settings_schema_source_get_default();
    for (size_t i = 0; source && i < G_N_ELEMENTS(SCHEMAS); i++) {
        GSettingsSchema *schema = g_settings_schema_source_lookup(source, SCHEMAS[i], TRUE);
        if (!schema) continue;
        GSettings *settings = g_settings_new(SCHEMAS[i]);
        for (size_t k = 0; k < G_N_ELEMENTS(KEYS); k++) {
            if (!g_settings_schema_has_key(schema, KEYS[k])) continue;
            char signal[64];
            snprintf(signal, sizeof signal, "changed::%s", KEYS[k]);
            g_signal_connect(settings, signal, G_CALLBACK(on_setting), NULL);
            // GSettings tells of a key's changes once it has been read.
            g_variant_unref(g_settings_get_value(settings, KEYS[k]));
        }
        g_settings_schema_unref(schema);
    }
    GDBusConnection *bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, NULL);
    if (bus)
        g_dbus_connection_signal_subscribe(bus, NULL, "org.xfce.Xfconf", "PropertyChanged", "/org/xfce/Xfconf", "xsettings",
                                           G_DBUS_SIGNAL_FLAGS_NONE, on_xfconf, NULL, NULL);
    GIOChannel *input = g_io_channel_unix_new(0);
    g_io_add_watch(input, G_IO_IN | G_IO_HUP | G_IO_ERR, gone, NULL);
    gtk_main();
}

int main(int argc, char **argv) {
    if (!gtk_init_check(&argc, &argv)) {
        fprintf(stderr, "sc-gtkstyle: no display\n");
        return 1;
    }
    if (argc == 2 && !strcmp(argv[1], "watch")) {
        watch();
        return 0;
    }
    gboolean colors_only = argc == 2 && !strcmp(argv[1], "colors");
    if (argc < 3 && !colors_only) {
        fprintf(stderr, "usage: sc-gtkstyle <scale> <button height> | sc-gtkstyle colors | sc-gtkstyle watch\n");
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
    // A file manager's side pane (GTK's places sidebar, Thunar's, Nemo's): the theme's `.sidebar` fill.
    GtkStyleContext *pane = node(window, GTK_TYPE_SCROLLED_WINDOW, "placessidebar", "sidebar");
    GdkRGBA side = sample(pane, 24, 24, 12, 12);
    if (side.alpha > 0) put_color("sidebar", side, background);
    g_object_unref(pane);
    // The scrollbars' sizes and colours: the app's theme takes them as it starts.
    scrollbars(window, background);
    if (colors_only) {
        printf("end\n");
        return 0;
    }

    indicator(window, GTK_TYPE_CHECK_BUTTON, "checkbutton", "check", "checkbox", gtk_render_check);
    indicator(window, GTK_TYPE_RADIO_BUTTON, "radiobutton", "radio", "radio", gtk_render_option);

    // A button four times as wide as high, cut into its ends (half its height
    // each) and a strip of its middle, which the app stretches to any width.
    for (int is_default = 0; is_default <= 1; is_default++) {
        GtkStyleContext *button = node(window, GTK_TYPE_BUTTON, "button", "text-button");
        if (is_default) gtk_style_context_add_class(button, GTK_STYLE_CLASS_DEFAULT);
        for (int state = NORMAL; state <= DISABLED; state++) {
            gtk_style_context_set_state(button, flags(state, FALSE));
            cairo_t *cr;
            cairo_surface_t *surface = canvas(button_height * 4, button_height, &cr);
            draw(button, cr, button_height * 4, button_height, NULL);
            cairo_destroy(cr);
            char name[64];
            snprintf(name, sizeof name, "button.%s.%s", is_default ? "default" : "plain", STATES[state]);
            cut(name, surface, 0, cairo_image_surface_get_width(surface), cairo_image_surface_get_height(surface) / 2);
            cairo_surface_destroy(surface);
        }
        g_object_unref(button);
    }
    fields(window, button_height);
    combos(window, background, button_height);
    tabs(window, background);
    tab_close(window);
    toggle(window, button_height);
    menus(window, background);
    printf("end\n");
    return 0;
}
