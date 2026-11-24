// sc-adwstyle: libadwaita's colours as GNOME's apps get them (Ubuntu's Yaru,
// high contrast, the accent, a user's gtk.css), light and dark, on stdout:
//   sc-adwstyle colors
// "sc-adwstyle 1", "high-contrast 0|1", "color <light|dark>.<name> <rrggbb>"
// lines (a colour with alpha over the window's), "end" last.

#include <adwaita.h>
#include <stdio.h>

// libadwaita's named colours the app takes.
static const char *const NAMES[] = {
    "window_bg_color",     "window_fg_color",          "view_bg_color",        "view_fg_color",
    "headerbar_bg_color",  "headerbar_fg_color",       "headerbar_backdrop_color",
    "sidebar_bg_color",    "sidebar_fg_color",         "sidebar_backdrop_color",
    "card_bg_color",       "popover_bg_color",         "popover_fg_color",     "dialog_bg_color",
    "accent_bg_color",     "accent_fg_color",          "accent_color",
    "destructive_bg_color", "destructive_color",       "error_color",          "borders",
};

static int channel(double value) {
    return (int)(value * 255 + 0.5);
}

// The scheme's colours, settled after the style manager switched to it.
static void colors(GtkWidget *widget, AdwColorScheme scheme, const char *name) {
    adw_style_manager_set_color_scheme(adw_style_manager_get_default(), scheme);
    while (g_main_context_iteration(NULL, FALSE)) {
    }
    G_GNUC_BEGIN_IGNORE_DEPRECATIONS
    GtkStyleContext *context = gtk_widget_get_style_context(widget);
    GdkRGBA window = {1, 1, 1, 1};
    gtk_style_context_lookup_color(context, "window_bg_color", &window);
    for (gsize i = 0; i < G_N_ELEMENTS(NAMES); i++) {
        GdkRGBA color;
        if (!gtk_style_context_lookup_color(context, NAMES[i], &color)) continue;
        // Translucent ones (the text's, borders) as drawn over the window.
        double r = color.red * color.alpha + window.red * (1 - color.alpha);
        double g = color.green * color.alpha + window.green * (1 - color.alpha);
        double b = color.blue * color.alpha + window.blue * (1 - color.alpha);
        printf("color %s.%s %02x%02x%02x\n", name, NAMES[i], channel(r), channel(g), channel(b));
    }
    G_GNUC_END_IGNORE_DEPRECATIONS
}

int main(int argc, char **argv) {
    if (argc != 2 || g_strcmp0(argv[1], "colors") != 0) {
        fprintf(stderr, "usage: sc-adwstyle colors\n");
        return 2;
    }
    adw_init();
    AdwStyleManager *manager = adw_style_manager_get_default();
    GtkWidget *window = gtk_window_new();
    printf("sc-adwstyle 1\n");
    printf("high-contrast %d\n", adw_style_manager_get_high_contrast(manager));
    colors(window, ADW_COLOR_SCHEME_FORCE_LIGHT, "light");
    colors(window, ADW_COLOR_SCHEME_FORCE_DARK, "dark");
    printf("end\n");
    gtk_window_destroy(GTK_WINDOW(window));
    return 0;
}
