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
 *
 * The browser hands each new web process its settings as it starts and sends a "config"
 * message when they change (src/window.rs): (gpc, https-only, sites allowed over HTTP).
 *
 * build.rs compiles it with the system's C compiler; tauri.conf.json ships it as a resource.
 */

#include <string.h>
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

static void
on_page_created (WebKitWebExtension *extension,
                 WebKitWebPage      *page,
                 gpointer            data)
{
    g_signal_connect (page, "send-request", G_CALLBACK (on_send_request), NULL);
}

static void
apply_config (GVariant *config)
{
    if (!config || !g_variant_is_of_type (config, G_VARIANT_TYPE ("(bbas)")))
        return;
    g_autoptr (GVariantIter) sites = NULL;
    const char *site;
    g_variant_get (config, "(bbas)", &send_gpc, &https_only, &sites);
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
    g_signal_connect (extension, "page-created", G_CALLBACK (on_page_created), NULL);
    g_signal_connect (extension, "user-message-received", G_CALLBACK (on_message), NULL);
}
