/*
 * The part of PEEPWICK Browser that runs inside WebKit's web processes, where each request a
 * page makes can still be changed before it leaves. WebKitGTK gives the browser itself no way
 * to add a header to every request; an extension in the web process can.
 *
 * It adds the Global Privacy Control header (Sec-GPC: 1) to every HTTP request while the
 * setting is on, as 0.1.x did. The browser hands each new web process the setting as it
 * starts and sends a "gpc" message when it changes (src/window.rs).
 *
 * build.rs compiles it with the system's C compiler; tauri.conf.json ships it as a resource.
 */

#include <webkit2/webkit-web-extension.h>

static gboolean send_gpc = FALSE;

static gboolean
on_send_request (WebKitWebPage     *page,
                 WebKitURIRequest  *request,
                 WebKitURIResponse *redirected_response,
                 gpointer           data)
{
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

static gboolean
on_message (WebKitWebExtension *extension,
            WebKitUserMessage  *message,
            gpointer            data)
{
    GVariant *parameters = webkit_user_message_get_parameters (message);
    if (g_strcmp0 (webkit_user_message_get_name (message), "gpc") != 0 || !parameters
        || !g_variant_is_of_type (parameters, G_VARIANT_TYPE_BOOLEAN))
        return FALSE;
    send_gpc = g_variant_get_boolean (parameters);
    return TRUE;
}

G_MODULE_EXPORT void
webkit_web_extension_initialize_with_user_data (WebKitWebExtension *extension,
                                                GVariant           *user_data)
{
    if (user_data && g_variant_is_of_type (user_data, G_VARIANT_TYPE_BOOLEAN))
        send_gpc = g_variant_get_boolean (user_data);
    g_signal_connect (extension, "page-created", G_CALLBACK (on_page_created), NULL);
    g_signal_connect (extension, "user-message-received", G_CALLBACK (on_message), NULL);
}
