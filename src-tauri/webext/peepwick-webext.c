/*
 * The part of PEEPWICK Browser that runs inside WebKit's web processes, where each request a
 * page makes can still be changed before it leaves. WebKitGTK gives the browser itself no way
 * to change a request, and its navigation decisions can't tell a tab's own page from a frame
 * in it; here both are possible.
 *
 * - Global Privacy Control: the Sec-GPC: 1 header on every HTTP request while the setting is
 *   on, as 0.1.x did.
 * - HTTPS-Only: a page's own HTTP address is asked for over HTTPS instead, before anything is
 *   sent. Frames keep their addresses, as in 0.1.x. The browser hears of each upgrade, to offer
 *   HTTP when the secure version fails (src/security.rs), and of a secure version that sends
 *   the page back to HTTP, which stops the load.
 * - The task manager: a page tells the browser which process it lives in ("pid").
 * - Media controls: every frame tells the browser when its video or audio starts or stops
 *   playing ("media"), and "media-toggle" pauses what plays in all of a page's frames, or
 *   plays again what it paused. This runs in a script world of its own, apart from the page's
 *   scripts.
 * - Shield counts: WebKit tells the browser nothing about the loads its content rules stop, but
 *   it writes a line about each to the page's console; those lines are gathered per page and
 *   sent to the browser ("blocked"), which counts them for the tab and the day.
 * - Autofill: in a page's main frame, another world of its own (webext/forms.js) notices
 *   sign-in and other form fields getting focus and forms being sent, and tells the browser
 *   ("forms"), with the page's address from WebKit, never from the page. The browser answers
 *   with what to fill ("forms").
 * - Page translation: the browser runs its scripts in a world named "peepwick-translate"
 *   (src/translation.rs). WebKitGTK gives a world the browser names a fresh start on every
 *   call unless it exists here, so it is made here: a page's text nodes and their originals stay
 *   in it between the calls that translate the page a block at a time and the one that undoes it.
 *
 * The browser hands each new web process its settings as it starts and sends a "config"
 * message when they change (src/window.rs): (gpc, https-only, sites allowed over HTTP).
 * Reader views' processes get no settings and answer only "pid".
 *
 * build.rs compiles it with the system's C compiler; tauri.conf.json ships it as a resource.
 */

#include <string.h>
#include <unistd.h>
#include <webkit2/webkit-web-extension.h>

static gboolean send_gpc = FALSE;
static gboolean https_only = FALSE;
/* Sites opened over HTTP on purpose this session; their subdomains go with them. */
static GPtrArray *http_allowed = NULL;

/* Per page: the address its current load is at, and the HTTP addresses it was moved off. */
#define PAGE_ADDRESS "peepwick-page-address"
#define PAGE_UPGRADED "peepwick-upgraded"

static char *
bare_host (const char *host)
{
    gsize start = host[0] == '[' ? 1 : 0;
    gsize end = strlen (host);
    while (end > start && (host[end - 1] == '.' || host[end - 1] == ']'))
        end--;
    return g_ascii_strdown (host + start, end - start);
}

/* Hosts on this computer or the local network, which can't get a certificate from a public
 * authority. The same rules as pages::is_local_host in src/pages.rs. */
static gboolean
is_local_host (const char *host)
{
    static const char *const suffixes[] = {
        ".localhost", ".local", ".lan", ".home", ".test", ".internal", ".intranet", ".corp",
        ".home.arpa", ".example", ".invalid",
    };
    g_autofree char *h = bare_host (host);
    if (!strchr (h, '.') && !strchr (h, ':'))
        return TRUE;
    for (gsize i = 0; i < G_N_ELEMENTS (suffixes); i++)
        if (g_str_has_suffix (h, suffixes[i]))
            return TRUE;
    g_autoptr (GInetAddress) address = g_inet_address_new_from_string (h);
    if (!address)
        return FALSE;
    const guint8 *bytes = g_inet_address_to_bytes (address);
    if (g_inet_address_get_family (address) == G_SOCKET_FAMILY_IPV4) {
        guint8 a = bytes[0], b = bytes[1];
        return a == 127 || a == 10 || a == 0 || (a == 169 && b == 254)
               || (a == 172 && b >= 16 && b <= 31) || (a == 192 && b == 168)
               || (a == 100 && b >= 64 && b <= 127);
    }
    guint16 first = (guint16) (bytes[0] << 8 | bytes[1]);
    return g_inet_address_get_is_loopback (address) || (first & 0xfe00) == 0xfc00
           || (first & 0xffc0) == 0xfe80;
}

static gboolean
is_http_allowed (const char *host)
{
    g_autofree char *h = bare_host (host);
    for (guint i = 0; http_allowed && i < http_allowed->len; i++) {
        const char *site = g_ptr_array_index (http_allowed, i);
        gsize length = strlen (h), site_length = strlen (site);
        if (g_str_equal (h, site)
            || (length > site_length && g_str_has_suffix (h, site) && h[length - site_length - 1] == '.'))
            return TRUE;
    }
    return FALSE;
}

/* Whether a request is for the page itself rather than a frame in it or something it uses.
 * WebKit names the destination only for trustworthy addresses (HTTPS, this computer); for
 * the rest, a page's own request is for the address the page is loading, and a redirect
 * follows from it. */
static gboolean
is_page_request (WebKitWebPage     *page,
                 WebKitURIRequest  *request,
                 WebKitURIResponse *redirected_response)
{
    SoupMessageHeaders *headers = webkit_uri_request_get_http_headers (request);
    const char *destination = headers ? soup_message_headers_get_one (headers, "Sec-Fetch-Dest") : NULL;
    if (destination)
        return g_str_equal (destination, "document");
    const char *page_uri = webkit_web_page_get_uri (page);
    if (!redirected_response)
        return g_strcmp0 (webkit_uri_request_get_uri (request), page_uri) == 0;
    const char *from = webkit_uri_response_get_uri (redirected_response);
    return g_strcmp0 (from, page_uri) == 0
           || g_strcmp0 (from, g_object_get_data (G_OBJECT (page), PAGE_ADDRESS)) == 0;
}

static void
tell_browser (WebKitWebPage *page, const char *name, GVariant *parameters)
{
    webkit_web_page_send_message_to_view (page, webkit_user_message_new (name, parameters), NULL,
                                          NULL, NULL);
}

/* HTTPS-Only for a page's own request. FALSE when the load has to stop: the secure version
 * sent the page back to an HTTP address it was already moved off. */
static gboolean
upgrade (WebKitWebPage *page, WebKitURIRequest *request, WebKitURIResponse *redirected_response)
{
    if (!redirected_response)
        g_object_set_data (G_OBJECT (page), PAGE_UPGRADED, NULL);
    const char *uri = webkit_uri_request_get_uri (request);
    if (!https_only || !g_str_has_prefix (uri, "http:"))
        return TRUE;
    g_autoptr (GUri) parsed = g_uri_parse (uri, G_URI_FLAGS_ENCODED, NULL);
    const char *host = parsed ? g_uri_get_host (parsed) : NULL;
    if (!host || !*host || is_local_host (host) || is_http_allowed (host))
        return TRUE;

    GPtrArray *upgraded = g_object_get_data (G_OBJECT (page), PAGE_UPGRADED);
    if (!upgraded) {
        upgraded = g_ptr_array_new_with_free_func (g_free);
        g_object_set_data_full (G_OBJECT (page), PAGE_UPGRADED, upgraded,
                                (GDestroyNotify) g_ptr_array_unref);
    }
    for (guint i = 0; i < upgraded->len; i++) {
        if (g_str_equal (g_ptr_array_index (upgraded, i), uri)) {
            tell_browser (page, "https-loop", g_variant_new_string (uri));
            return FALSE;
        }
    }
    /* An explicit :80 belongs to HTTP; HTTPS on that port would never answer. */
    gint port = g_uri_get_port (parsed);
    g_autoptr (GUri) secure = g_uri_build (G_URI_FLAGS_ENCODED, "https", g_uri_get_userinfo (parsed),
                                           host, port == 80 ? -1 : port, g_uri_get_path (parsed),
                                           g_uri_get_query (parsed), g_uri_get_fragment (parsed));
    g_autofree char *secure_uri = g_uri_to_string (secure);
    g_ptr_array_add (upgraded, g_strdup (uri));
    tell_browser (page, "https-upgraded", g_variant_new ("(ss)", uri, secure_uri));
    webkit_uri_request_set_uri (request, secure_uri);
    return TRUE;
}

static gboolean
on_send_request (WebKitWebPage     *page,
                 WebKitURIRequest  *request,
                 WebKitURIResponse *redirected_response,
                 gpointer           data)
{
    if (is_page_request (page, request, redirected_response)) {
        if (!upgrade (page, request, redirected_response))
            return TRUE;
        g_object_set_data_full (G_OBJECT (page), PAGE_ADDRESS,
                                g_strdup (webkit_uri_request_get_uri (request)), g_free);
    }
    if (send_gpc) {
        /* NULL for requests that aren't HTTP. */
        SoupMessageHeaders *headers = webkit_uri_request_get_http_headers (request);
        if (headers)
            soup_message_headers_replace (headers, "Sec-GPC", "1");
    }
    return FALSE;
}

/* ── Media ───────────────────────────────────────────────────────────────────────── */

static WebKitWebExtension *web_extension = NULL;
static WebKitScriptWorld *media_world = NULL;

/* Per page: weak references to its frames, the main frame first. */
#define PAGE_FRAMES "peepwick-frames"

/* Set up in every frame as its window is: reports whether anything plays in the frame when
 * that changes, and the three calls media-toggle makes. */
static const char MEDIA_JS[] =
    "(() => {\n"
    "  const report = globalThis.__peepwickMediaReport\n"
    "  delete globalThis.__peepwickMediaReport\n"
    "  const all = () => document.querySelectorAll('video, audio')\n"
    "  const playing = () => [...all()].some((m) => !m.paused && !m.ended)\n"
    "  let said = false\n"
    "  const tell = (now) => {\n"
    "    if (now !== said) {\n"
    "      said = now\n"
    "      report(now)\n"
    "    }\n"
    "  }\n"
    "  for (const type of ['play', 'pause', 'ended', 'emptied']) addEventListener(type, () => tell(playing()), true)\n"
    "  addEventListener('pagehide', () => tell(false), true)\n"
    "  const ours = new Set()\n"
    "  globalThis.__peepwickPause = () => {\n"
    "    let paused = false\n"
    "    for (const m of all()) {\n"
    "      if (m.paused || m.ended) continue\n"
    "      m.pause()\n"
    "      ours.add(m)\n"
    "      paused = true\n"
    "    }\n"
    "    return paused\n"
    "  }\n"
    "  globalThis.__peepwickResume = () => {\n"
    "    let resumed = false\n"
    "    for (const m of ours) {\n"
    "      if (!m.isConnected || !m.paused) continue\n"
    "      m.play().catch(() => {})\n"
    "      resumed = true\n"
    "    }\n"
    "    ours.clear()\n"
    "    return resumed\n"
    "  }\n"
    "  globalThis.__peepwickPlayFirst = () => {\n"
    "    const m = all()[0]\n"
    "    if (m) m.play().catch(() => {})\n"
    "    return !!m\n"
    "  }\n"
    "})()\n";

static void
media_report (gboolean playing, gpointer data)
{
    WebKitWebPage *page = webkit_web_extension_get_page (web_extension, *(guint64 *) data);
    if (page)
        tell_browser (page, "media", g_variant_new_boolean (playing));
}

static void
free_weak_ref (gpointer ref)
{
    g_weak_ref_clear (ref);
    g_free (ref);
}

/* WebKit doesn't list a page's frames: each is noted when its window is set up, and drops out
 * when it goes away. A frame keeps its place when it moves on to another page. */
static void
remember_frame (WebKitWebPage *page, WebKitFrame *frame)
{
    GPtrArray *frames = g_object_get_data (G_OBJECT (page), PAGE_FRAMES);
    if (!frames) {
        frames = g_ptr_array_new_with_free_func (free_weak_ref);
        g_object_set_data_full (G_OBJECT (page), PAGE_FRAMES, frames,
                                (GDestroyNotify) g_ptr_array_unref);
    }
    for (guint i = frames->len; i-- > 0;) {
        g_autoptr (WebKitFrame) known = g_weak_ref_get (g_ptr_array_index (frames, i));
        if (known == frame)
            return;
        if (!known)
            g_ptr_array_remove_index (frames, i);
    }
    GWeakRef *ref = g_new0 (GWeakRef, 1);
    g_weak_ref_init (ref, frame);
    g_ptr_array_add (frames, ref);
}

static void
on_window_object_cleared (WebKitScriptWorld *world,
                          WebKitWebPage     *page,
                          WebKitFrame       *frame,
                          gpointer           data)
{
    g_autoptr (JSCContext) context = webkit_frame_get_js_context_for_script_world (frame, world);
    guint64 *page_id = g_new (guint64, 1);
    *page_id = webkit_web_page_get_id (page);
    g_autoptr (JSCValue) report = jsc_value_new_function (context, NULL, G_CALLBACK (media_report),
                                                         page_id, g_free, G_TYPE_NONE, 1,
                                                         G_TYPE_BOOLEAN);
    jsc_context_set_value (context, "__peepwickMediaReport", report);
    g_autoptr (JSCValue) done = jsc_context_evaluate (context, MEDIA_JS, -1);
    remember_frame (page, frame);
}

/* Makes one of MEDIA_JS's calls in the page's frames: in all of them, or until one does
 * something. TRUE when one did. */
static gboolean
in_frames (WebKitWebPage *page, const char *call, gboolean all)
{
    GPtrArray *frames = g_object_get_data (G_OBJECT (page), PAGE_FRAMES);
    gboolean done = FALSE;
    for (guint i = 0; frames && i < frames->len && (all || !done); i++) {
        g_autoptr (WebKitFrame) frame = g_weak_ref_get (g_ptr_array_index (frames, i));
        if (!frame)
            continue;
        g_autoptr (JSCContext) context =
            webkit_frame_get_js_context_for_script_world (frame, media_world);
        g_autoptr (JSCValue) result = jsc_context_evaluate (context, call, -1);
        jsc_context_clear_exception (context);
        if (jsc_value_to_boolean (result))
            done = TRUE;
    }
    return done;
}

/* Pauses what plays in the page, or plays again what this paused; with neither, the page's
 * first video or audio. */
static gboolean
toggle_media (WebKitWebPage *page)
{
    return in_frames (page, "__peepwickPause()", TRUE)
           || in_frames (page, "__peepwickResume()", TRUE)
           || in_frames (page, "__peepwickPlayFirst()", FALSE);
}

/* ── Autofill ───────────────────────────────────────────────────────────────── */

static WebKitScriptWorld *forms_world = NULL;
/* Kept for the process's life, nothing more: see the comment at the top. */
static WebKitScriptWorld *translate_world = NULL;
/* Tabs' processes get settings; reader views' don't, and have no sign-in forms. */
static gboolean has_config = FALSE;

/* Set up in a page's main frame as its window is: webext/forms.js, which build.rs turns into
 * FORMS_JS. It tells the browser about sign-in and other form fields getting focus or losing it,
 * typing in them, keys that move through the browser's list while it's open, and forms being
 * sent; it fills in what the browser sends back. */
#include "forms-js.h"

static void
forms_send (const char *type, const char *payload, gpointer data)
{
    WebKitWebPage *page = webkit_web_extension_get_page (web_extension, *(guint64 *) data);
    if (!page || !type || !payload)
        return;
    /* The address is WebKit's, of the main frame: the only frame this runs in. */
    const char *uri = webkit_web_page_get_uri (page);
    tell_browser (page, "forms", g_variant_new ("(sss)", type, payload, uri ? uri : ""));
}

static void
on_forms_window_cleared (WebKitScriptWorld *world,
                         WebKitWebPage     *page,
                         WebKitFrame       *frame,
                         gpointer           data)
{
    if (!has_config || !webkit_frame_is_main_frame (frame))
        return;
    g_autoptr (JSCContext) context = webkit_frame_get_js_context_for_script_world (frame, world);
    guint64 *page_id = g_new (guint64, 1);
    *page_id = webkit_web_page_get_id (page);
    g_autoptr (JSCValue) send = jsc_value_new_function (context, NULL, G_CALLBACK (forms_send),
                                                       page_id, g_free, G_TYPE_NONE, 2,
                                                       G_TYPE_STRING, G_TYPE_STRING);
    jsc_context_set_value (context, "__peepwickFormsSend", send);
    g_autoptr (JSCValue) done = jsc_context_evaluate (context, FORMS_JS, -1);
}

/* What the browser sends to fill in, or the state of its list, to the main frame's world. */
static void
forms_receive (WebKitWebPage *page, const char *json)
{
    WebKitFrame *frame = webkit_web_page_get_main_frame (page);
    if (!frame)
        return;
    g_autoptr (JSCContext) context =
        webkit_frame_get_js_context_for_script_world (frame, forms_world);
    g_autoptr (JSCValue) receive = jsc_context_get_value (context, "__peepwickFormsReceive");
    if (jsc_value_is_function (receive)) {
        g_autoptr (JSCValue) result =
            jsc_value_function_call (receive, G_TYPE_STRING, json, G_TYPE_NONE);
    }
    jsc_context_clear_exception (context);
}

/* ── Messages from the browser to one page ───────────────────────────────────────── */

static gboolean
on_page_message (WebKitWebPage     *page,
                 WebKitUserMessage *message,
                 gpointer           data)
{
    const char *name = webkit_user_message_get_name (message);
    GVariant *reply;
    if (g_strcmp0 (name, "pid") == 0)
        reply = g_variant_new_int32 (getpid ());
    else if (g_strcmp0 (name, "media-toggle") == 0)
        reply = g_variant_new_boolean (toggle_media (page));
    else if (g_strcmp0 (name, "forms") == 0) {
        GVariant *parameters = webkit_user_message_get_parameters (message);
        if (parameters && g_variant_is_of_type (parameters, G_VARIANT_TYPE_STRING))
            forms_receive (page, g_variant_get_string (parameters, NULL));
        reply = g_variant_new_boolean (TRUE);
    } else
        return FALSE;
    webkit_user_message_send_reply (message, webkit_user_message_new (name, reply));
    return TRUE;
}

/* ── Shield counts ─────────────────────────────────────────────────────────────────── */

/* The line WebKit writes for each load a content rule stops, frames included (the first
 * address is then still the tab's page):
 *   "Content blocker prevented frame displaying <page> from loading a resource from <url>" */
#define BLOCKED_START "Content blocker prevented frame displaying "
#define BLOCKED_MIDDLE " from loading a resource from "
#define PAGE_BLOCKED "peepwick-blocked"
/* Pairs of addresses kept between two messages; more in a quarter of a second are left out. */
#define BLOCKED_MAX 500

/* Sends what a page gathered: (page, resource) pairs. */
static gboolean
send_blocked (gpointer data)
{
    WebKitWebPage *page = webkit_web_extension_get_page (web_extension, *(guint64 *) data);
    if (!page)
        return G_SOURCE_REMOVE;
    GPtrArray *pending = g_object_steal_data (G_OBJECT (page), PAGE_BLOCKED);
    if (!pending)
        return G_SOURCE_REMOVE;
    GVariantBuilder builder;
    g_variant_builder_init (&builder, G_VARIANT_TYPE ("a(ss)"));
    for (guint i = 0; i + 1 < pending->len; i += 2)
        g_variant_builder_add (&builder, "(ss)", g_ptr_array_index (pending, i),
                               g_ptr_array_index (pending, i + 1));
    tell_browser (page, "blocked", g_variant_builder_end (&builder));
    g_ptr_array_unref (pending);
    return G_SOURCE_REMOVE;
}

/* The console signal is deprecated, but it is the only place WebKitGTK says what it blocked. */
G_GNUC_BEGIN_IGNORE_DEPRECATIONS
static void
on_console_message (WebKitWebPage *page, WebKitConsoleMessage *message, gpointer data)
{
    const char *text = webkit_console_message_get_text (message);
    if (!has_config || !text || !g_str_has_prefix (text, BLOCKED_START))
        return;
    const char *document = text + strlen (BLOCKED_START);
    const char *middle = strstr (document, BLOCKED_MIDDLE);
    if (!middle)
        return;
    GPtrArray *pending = g_object_get_data (G_OBJECT (page), PAGE_BLOCKED);
    if (!pending) {
        pending = g_ptr_array_new_with_free_func (g_free);
        g_object_set_data_full (G_OBJECT (page), PAGE_BLOCKED, pending,
                                (GDestroyNotify) g_ptr_array_unref);
        guint64 *id = g_new (guint64, 1);
        *id = webkit_web_page_get_id (page);
        g_timeout_add_full (G_PRIORITY_DEFAULT, 250, send_blocked, id, g_free);
    }
    if (pending->len >= BLOCKED_MAX * 2)
        return;
    g_ptr_array_add (pending, g_strndup (document, middle - document));
    g_ptr_array_add (pending, g_strdup (middle + strlen (BLOCKED_MIDDLE)));
}
G_GNUC_END_IGNORE_DEPRECATIONS

static void
on_page_created (WebKitWebExtension *extension,
                 WebKitWebPage      *page,
                 gpointer            data)
{
    g_signal_connect (page, "send-request", G_CALLBACK (on_send_request), NULL);
    g_signal_connect (page, "user-message-received", G_CALLBACK (on_page_message), NULL);
    g_signal_connect (page, "console-message-sent", G_CALLBACK (on_console_message), NULL);
}

static void
apply_config (GVariant *config)
{
    if (!config || !g_variant_is_of_type (config, G_VARIANT_TYPE ("(bbas)")))
        return;
    g_autoptr (GVariantIter) sites = NULL;
    const char *site;
    g_variant_get (config, "(bbas)", &send_gpc, &https_only, &sites);
    has_config = TRUE;
    g_clear_pointer (&http_allowed, g_ptr_array_unref);
    http_allowed = g_ptr_array_new_with_free_func (g_free);
    while (g_variant_iter_next (sites, "&s", &site))
        g_ptr_array_add (http_allowed, bare_host (site));
}

static gboolean
on_message (WebKitWebExtension *extension,
            WebKitUserMessage  *message,
            gpointer            data)
{
    if (g_strcmp0 (webkit_user_message_get_name (message), "config") != 0)
        return FALSE;
    apply_config (webkit_user_message_get_parameters (message));
    return TRUE;
}

G_MODULE_EXPORT void
webkit_web_extension_initialize_with_user_data (WebKitWebExtension *extension,
                                                GVariant           *user_data)
{
    apply_config (user_data);
    web_extension = extension;
    media_world = webkit_script_world_new_with_name ("peepwick-media");
    g_signal_connect (media_world, "window-object-cleared", G_CALLBACK (on_window_object_cleared),
                      NULL);
    forms_world = webkit_script_world_new_with_name ("peepwick-forms");
    g_signal_connect (forms_world, "window-object-cleared", G_CALLBACK (on_forms_window_cleared),
                      NULL);
    translate_world = webkit_script_world_new_with_name ("peepwick-translate");
    g_signal_connect (extension, "page-created", G_CALLBACK (on_page_created), NULL);
    g_signal_connect (extension, "user-message-received", G_CALLBACK (on_message), NULL);
}
