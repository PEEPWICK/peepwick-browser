# PEEPWICK Browser

**A calm, private browser that guards your attention. First, we look.**

> **Alpha.** PEEPWICK Browser is in alpha: features may change and you may run into bugs.
> Releases are tagged with an `a` (v0.1.3a). Please [report what you find](https://github.com/PEEPWICK/peepwick-browser/issues/new?template=bug_report.yml); Settings › Report a bug fills in the details for you.

PEEPWICK Browser is built on Chromium (through [Electron](https://www.electronjs.org/)) and shaped
by one idea from the PEEPWICK brand: attention is the scarcest thing online. It blocks ads and
trackers, keeps your spaces apart and helps you focus, inside a quiet black-and-white frame that
lets the pages you visit keep their own colour.

![New tab](docs/screenshots/newtab.png)

[Türkçe aşağıda](#türkçe)

## Features

- **Shield.** Ads and trackers are blocked with the EasyList, EasyPrivacy and uBlock Origin lists
  (through Ghostery's engine). Third-party cookies are blocked and the Global Privacy Control
  signal is sent. The shield shows what it blocked on each page and can be switched off per site.
  A strict level also hides cookie notices.
- **Focus.** Start a 25, 50 or 90-minute session and the sites you choose wait behind a calm gate
  until it ends. You can still open one for five minutes after a short pause. Site notifications
  are silenced while you focus.
- **Spaces and vertical tabs.** Tabs live in a sidebar, grouped into spaces (work, personal…).
  Pin tabs, drag them between spaces, open two side by side, and let unused tabs sleep.
- **Reader view.** One click turns an article into clean type set in Newsreader, shown with
  scripts disabled.
- **Bookmarks.** A bookmarks bar with folders, a full bookmark manager (folder tree, search,
  drag and drop, multi-select, undo) and the star in the address bar. Import from Chrome,
  Chromium, Brave, Edge, Vivaldi, Opera, Firefox, LibreWolf, Zen and Floorp (Snap and Flatpak
  installs included), or from an HTML file; export to HTML. Imports read the other browser's
  files on this computer and bring the site icons along.
- **Passwords.** A built-in password manager: it offers to save or update a password after you
  sign in, suggests saved logins in sign-in forms (only on the same site, filled only when you pick
  one), suggests a strong password on sign-up forms, and has a passwords page with a local check for
  weak and reused passwords. Passwords are encrypted with your system keyring (GNOME Keyring,
  KWallet). Addresses and form entries are remembered and suggested too; card numbers never are.
- **Import from other browsers:** bookmarks, passwords, history, cookies (stay signed in) and form
  data from Chrome, Chromium, Brave, Edge, Vivaldi, Opera, Firefox, LibreWolf, Zen and Floorp
  (Snap and Flatpak too), including Firefox profiles with a primary password; passwords also from
  a CSV export (Chrome, Firefox, Bitwarden, 1Password, LastPass, KeePassXC).
- **Site security.** The button at the left of the address bar shows whether the connection is
  secure, based on Chromium's own certificate check rather than the `https` in the address. Open
  it to see the certificate (who issued it, the domains it covers, validity, fingerprints, chain),
  the site's permissions, and to delete the site's cookies and data.
- **HTTPS-Only mode.** Sites open over an encrypted connection; when one has no secure version,
  the browser asks before continuing over HTTP. Addresses on your own network are left alone.
- **Dangerous site warnings.** Phishing and malware sites are stopped behind a warning. The lists
  (from the malware-filter project: URLhaus, OpenPhish, PhishTank, IPThreat) are downloaded every
  12 hours and checked on this computer; the addresses you visit are never sent anywhere.
- **Fingerprinting protection.** Canvas, WebGL and audio readouts get tiny changes that differ for
  every site and every session, the graphics card model is hidden and the processor count rounded,
  so sites can't recognise you across sites.
- **Cookie banners rejected.** Consent notices from common tools (OneTrust, Cookiebot, Didomi,
  Usercentrics, Quantcast and more), and other clearly marked ones, get their "reject" button
  pressed for you.
- **Reading list.** Save pages to read later; articles are kept on this computer and can be read
  offline. **Read aloud** speaks reader view and saved articles with your system's voices.
- **Page translation** on this computer with Bergamot, the engine Firefox's translations grew out
  of. A model (about 20 MB) is downloaded once per language pair after you agree; the page's text
  never leaves the browser. For now the Bergamot project's models cover German, French, Spanish,
  Italian, Portuguese, Dutch, Polish, Czech, Russian, Ukrainian, Bulgarian, Estonian, Persian,
  Icelandic and Norwegian (into English, and English into most of them); there is no Turkish model yet.
- **Media controls and picture-in-picture.** Play, pause and mute what plays in any tab from the
  toolbar, and pop a video out into its own small window.
- **Per-site zoom.** Zoom is remembered for each site; set a default zoom and font size.
- **Task manager** (<kbd>Shift</kbd>+<kbd>Esc</kbd>): memory and processor use per tab and process;
  end a page that hangs.
- **Downloads.** A full downloads page (<kbd>Ctrl</kbd>+<kbd>J</kbd>) with progress, speed and time
  left; pause, resume, cancel, retry, open, show in folder, move to trash; searchable history.
- **Clearing browsing data** (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Delete</kbd>) for a time range,
  per site from the history page, or every time the browser closes.
- **Encrypted backup.** Settings, bookmarks, spaces, focus history, site permissions, the extension
  list and, if you choose, open tabs and browsing history go into one password-protected file
  (scrypt + AES-256-GCM). Restore it on another computer; it merges with what is there.
- **Secure DNS** (DNS over HTTPS), automatic or through Cloudflare, Quad9, Mullvad or Google.
- **Report a bug.** Settings › Report a bug shows a short report (version, system, error codes,
  never addresses or file names) to copy or send to GitHub's bug form in one click.
- **Command bar.** <kbd>Ctrl</kbd>+<kbd>K</kbd> finds open tabs, bookmarks, history and commands.
- **Chrome extensions.** Install extensions from the Chrome Web Store. Most popular extensions
  work; some APIs are not available yet (see limitations).
- **Private windows** keep nothing after they close.
- **Automatic updates** from GitHub Releases.
- Turkish and English interface.

![Focus gate](docs/screenshots/focus.png)

## Install

Download the latest release from the
[Releases page](https://github.com/PEEPWICK/peepwick-browser/releases):

- **AppImage** (any distribution): make it executable and run it.

  ```bash
  chmod +x peepwick-browser-*.AppImage
  ./peepwick-browser-*.AppImage
  ```

- **.deb** (Ubuntu, Debian, Zorin, Mint…):

  ```bash
  sudo apt install ./peepwick-browser-*-amd64.deb
  ```

Both update themselves: the browser checks GitHub Releases at startup and every four hours,
downloads new versions in the background and installs them on restart. The .deb update asks for
your password through the system dialog.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>T</kbd> / <kbd>Ctrl</kbd>+<kbd>W</kbd> | New tab / close tab |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | Reopen closed tab |
| <kbd>Ctrl</kbd>+<kbd>L</kbd> | Address bar |
| <kbd>Ctrl</kbd>+<kbd>K</kbd> | Command bar |
| <kbd>Ctrl</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Go to tab |
| <kbd>Alt</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Go to space |
| <kbd>Ctrl</kbd>+<kbd>B</kbd> | Show or hide the sidebar |
| <kbd>Ctrl</kbd>+<kbd>D</kbd> | Bookmark this page |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd> | Bookmark all tabs of the space |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>B</kbd> | Show or hide the bookmarks bar |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>O</kbd> | Bookmark manager |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | Side by side with the previous tab |
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>R</kbd> | Reader view |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Find on page |
| <kbd>Ctrl</kbd>+<kbd>H</kbd> / <kbd>Ctrl</kbd>+<kbd>J</kbd> | History / downloads |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Delete</kbd> | Clear browsing data |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> | Private window |
| <kbd>Shift</kbd>+<kbd>Esc</kbd> | Task manager |

## What the browser sends, and where

PEEPWICK Browser has no telemetry and no account. It talks to these services on its own:

| Service | Why | When |
| --- | --- | --- |
| `raw.githubusercontent.com` | Filter lists for the shield | First start, then every three days |
| `malware-filter.gitlab.io` (or its mirror `curbengh.github.io`) | Dangerous site lists | First start, then every 12 hours; off with the setting |
| `storage.googleapis.com/bergamot-models-sandbox` | Translation models | Only when you translate from a new language, after you agree |
| `github.com` (Releases) | Update checks and downloads | At startup and every four hours |
| Your search engine | Search suggestions | Only if you turn suggestions on (off by default) |
| Chrome Web Store | Installing and updating extensions | Only if you install extensions |
| Your secure DNS provider | Looking up site addresses | Only if you pick one (automatic uses your system's provider) |

Pages you visit load as in any browser. Favicons are fetched once by the browser and kept locally.
Importing reads the other browser's files on this computer; nothing leaves it. For Chromium-based
browsers, the key their passwords and cookies are encrypted with is read from your system keyring.
The error log behind "Report a bug" stays on this computer (in a file only if you turn that on);
it reaches GitHub only when you send a report yourself. Backups are files you keep; no cloud.

## Build from source

Requirements: Node.js 24 and npm 11 on Linux.

```bash
npm ci
npm run dev        # run with hot reload
npm run typecheck
npm test
npm run dist       # build an AppImage and a .deb into dist/
```

`npm run icons` regenerates the app icons from the brand geometry, and `npm run social` the
social preview card in `docs/`.

## Releasing

1. Bump the version: `npm run release -- patch` (or `minor`, `major`). This commits and tags
   the release. `package.json` keeps a SemVer version (`0.1.3`) for the build tools and the
   updater; `releaseStage` adds the suffix people see: `alpha` → `v0.1.3a`, `beta` → `v0.1.3b`,
   `stable` → `v0.1.3`.
2. Push the commit and the tag: `git push --follow-tags`.
3. The **Release** workflow builds the AppImage and the .deb and publishes a GitHub release.
   Installed browsers see it within four hours, or right away from *Settings › About*.

## How it is built

- `src/main`: the Electron main process. Each window has one UI view and one
  `WebContentsView` per visible tab. All tabs are sandboxed. A small page preload runs in an
  isolated world for autofill, cookie banners and fingerprinting protection; it offers pages no
  bridge. Built-in pages (new tab, settings, history, extensions), error screens and the focus
  gate are drawn by the UI itself, so web content never reaches a privileged page.
- `src/main/network.ts`: one request pipeline per session (Electron allows a single listener),
  running the focus gate, dangerous site checks, HTTPS-Only upgrades, the shield, third-party
  cookie blocking and Global Privacy Control.
- `src/renderer`: the UI in React, styled with the PEEPWICK brand tokens: Gece, Beyaz, warm
  greys and one Fitil amber accent; Archivo, Newsreader and IBM Plex Mono.
- `src/shared`: types, the IPC contract, translations and pure helpers shared by both sides.
- History is stored with Node's built-in `node:sqlite`; the app ships no native modules.

## Known limitations

- Linux only for now.
- Netflix, Spotify and other DRM-protected video don't play yet. Widevine needs castlabs' Electron
  build, which has no stable release for Electron 45 yet; it comes when that does.
- Extensions that rely on `chrome.webRequest` don't receive events, because the shield owns the
  request pipeline. Extensions run in normal windows only.

## License

PEEPWICK Browser is free software under the [GNU General Public License v3.0](LICENSE).
It includes Chromium and Electron (BSD and MIT), Ghostery's adblocker (MPL-2.0),
electron-chrome-extensions (GPL-3.0), Mozilla Readability (Apache-2.0), bergamot-translator
(MPL-2.0), and the Archivo,
Newsreader and IBM Plex Mono typefaces (SIL Open Font License 1.1).

The PEEPWICK name and the Double-Eye W mark identify PEEPWICK and are not part of the GPL grant.
If you distribute a modified version, please give it your own name and icon.

---

## Türkçe

**Dikkatinizi koruyan, sakin ve gizli bir tarayıcı. Önce bakarız.**

> **Alpha.** PEEPWICK Browser alpha aşamasında: özellikler değişebilir, hatalarla
> karşılaşabilirsiniz. Sürüm etiketleri `a` ile biter (v0.1.3a).

PEEPWICK Browser, Chromium tabanlıdır (Electron ile) ve PEEPWICK markasının bir fikrine dayanır:
dijital dünyanın en kıt kaynağı dikkattir. Reklamları ve izleyicileri engeller, alanlarınızı
ayrı tutar, odaklanmanıza yardım eder. Siyah-beyaz, sakin bir çerçeve sunar; gezdiğiniz
sayfalar kendi renkleriyle kalır.

### Özellikler

- **Kalkan:** EasyList, EasyPrivacy ve uBlock Origin listeleriyle reklam ve izleyici engelleme,
  üçüncü taraf çerez engelleme, Global Privacy Control sinyali. Her sayfada neyin engellendiğini
  gösterir; site bazında kapatılabilir. "Sıkı" düzey çerez bildirimlerini de gizler.
- **Odak:** 25, 50 ya da 90 dakikalık oturumlar. Seçtiğiniz siteler oturum boyunca sakin bir
  ekranın arkasında bekler. Kısa bir beklemeden sonra bir siteyi beş dakikalığına açabilirsiniz.
- **Alanlar ve dikey sekmeler:** Sekmeler kenar çubuğunda, iş ve kişisel gibi alanlarda durur.
  Sabitleme, alanlar arası sürükleme, yan yana görünüm ve kullanılmayan sekmeleri uyutma.
- **Okuma modu:** Makaleyi Newsreader ile dizilmiş, betikleri kapalı temiz bir sayfaya çevirir.
- **Yer imleri:** Klasörlü yer imi çubuğu, tam bir yer imi yöneticisi (klasör ağacı, arama,
  sürükle-bırak, çoklu seçim, geri alma) ve adres çubuğundaki yıldız (<kbd>Ctrl</kbd>+<kbd>D</kbd>).
  Chrome, Chromium, Brave, Edge, Vivaldi, Opera, Firefox, LibreWolf, Zen ve Floorp'tan (Snap ve
  Flatpak kurulumları dahil) ya da bir HTML dosyasından içe aktarma; HTML olarak dışa aktarma.
  İçe aktarma yalnızca bu bilgisayardaki dosyaları okur, site simgelerini de getirir.
- **Site güvenliği:** Adres çubuğunun solundaki düğme bağlantının güvenli olup olmadığını,
  adresteki `https`'e değil Chromium'un sertifika denetimine bakarak gösterir. Açınca sertifikayı
  (veren kuruluş, kapsadığı alan adları, geçerlilik, parmak izleri, zincir), sitenin izinlerini
  görebilir, sitenin çerezlerini ve verilerini silebilirsiniz.
- **İndirmeler:** İlerleme, hız ve kalan süreyle tam bir indirmeler sayfası (<kbd>Ctrl</kbd>+<kbd>J</kbd>);
  duraklatma, sürdürme, iptal, yeniden deneme, açma, klasörde gösterme, çöp kutusuna taşıma,
  aranabilir geçmiş.
- **HTTPS-Only modu:** Siteler şifreli bağlantıyla açılır; güvenli sürümü olmayan bir sitede HTTP
  ile devam etmeden önce sorulur. Kendi ağınızdaki adresler etkilenmez.
- **Zararlı site uyarısı:** Oltalama ve zararlı yazılım siteleri bir uyarının arkasında durur.
  Listeler (malware-filter projesi: URLhaus, OpenPhish, PhishTank, IPThreat) 12 saatte bir indirilir
  ve bilgisayarınızda denetlenir; ziyaret ettiğiniz adresler hiçbir yere gönderilmez.
- **Parmak izi koruması:** Canvas, WebGL ve ses değerlerine her site ve her oturum için farklı,
  fark edilmeyecek kadar küçük değişiklikler katılır; ekran kartı modeli gizlenir, işlemci sayısı
  yuvarlanır. Siteler sizi başka sitelerde tanıyamaz.
- **Çerez bildirimlerini reddetme:** Yaygın onay araçlarının (OneTrust, Cookiebot, Didomi,
  Usercentrics, Quantcast ve diğerleri) ve açıkça işaretli diğer bildirimlerin "Reddet" düğmesine
  sizin yerinize basılır.
- **Okuma listesi:** Sayfaları sonra okumak için kaydedin; makaleler bilgisayarınızda saklanır,
  internet olmadan da okunur. **Sesli okuma**, okuma modunu ve kayıtlı makaleleri sistem sesleriyle okur.
- **Sayfa çevirisi:** Firefox'un çevirisinin temelindeki Bergamot motoruyla, bilgisayarınızda.
  Her dil çifti için bir model (yaklaşık 20 MB) siz onaylayınca bir kez indirilir; sayfanın metni
  tarayıcıdan çıkmaz. Şimdilik Bergamot projesinin modelleri Almanca, Fransızca, İspanyolca,
  İtalyanca, Portekizce, Felemenkçe, Lehçe, Çekçe, Rusça, Ukraynaca, Bulgarca, Estonca, Farsça,
  İzlandaca ve Norveççeyi kapsar (İngilizceye, çoğunda İngilizceden de); Türkçe model henüz yok.
- **Medya denetimleri ve resim içinde resim:** Herhangi bir sekmede çalanı araç çubuğundan oynatın,
  duraklatın, sesini kapatın; videoyu ayrı, küçük bir pencereye alın.
- **Site başına yakınlaştırma:** Yakınlaştırma her site için hatırlanır; varsayılan yakınlaştırma
  ve yazı boyutu seçilebilir.
- **Görev yöneticisi** (<kbd>Shift</kbd>+<kbd>Esc</kbd>): sekme ve işlem başına bellek ve işlemci
  kullanımı; takılan bir sayfayı sonlandırma.
- **Tarama verilerini temizleme** (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Delete</kbd>): zaman
  aralığıyla, geçmiş sayfasından site bazında ya da tarayıcı her kapandığında.
- **Şifreli yedek:** Ayarlar, yer imleri, alanlar, odak geçmişi, site izinleri, eklenti listesi
  ve isterseniz açık sekmeler ile tarama geçmişi, parolayla korunan tek bir dosyaya girer (scrypt +
  AES-256-GCM). Başka bir bilgisayarda geri yükleyin; mevcut verilerle birleştirilir.
- **Güvenli DNS** (DNS over HTTPS): otomatik ya da Cloudflare, Quad9, Mullvad veya Google ile.
- **Hata bildir:** Ayarlar › Hata bildir, kısa bir rapor gösterir (sürüm, sistem, hata kodları;
  adresler ve dosya adları asla girmez). Tek tıkla kopyalayın ya da GitHub'daki hata formuna gönderin.
- **Parolalar:** Yerleşik parola yöneticisi. Oturum açınca parolayı kaydetmeyi ya da güncellemeyi
  önerir, giriş formlarında kayıtlı parolaları önerir (yalnızca aynı sitede, siz seçince doldurur),
  kayıt formlarında güçlü parola önerir. Parolalar sayfasında zayıf ve tekrarlanan parolalar yerel
  olarak denetlenir. Parolalar sistem anahtarlığıyla (GNOME Anahtarlık, KWallet) şifrelenir. Adresler
  ve form girdileri de hatırlanıp önerilir; kart numaraları asla kaydedilmez.
- **Başka tarayıcılardan aktarma:** Chrome, Chromium, Brave, Edge, Vivaldi, Opera, Firefox, LibreWolf,
  Zen ve Floorp'tan (Snap ve Flatpak dahil) yer imleri, parolalar, geçmiş, çerezler (oturumlar açık
  kalır) ve form bilgileri; ana parolalı Firefox profilleri de. Parolalar ayrıca CSV'den (Chrome,
  Firefox, Bitwarden, 1Password, LastPass, KeePassXC).
- **Komut çubuğu:** <kbd>Ctrl</kbd>+<kbd>K</kbd> ile açık sekmeler, yer imleri, geçmiş ve komutlar.
- **Chrome eklentileri:** Chrome Web Mağazası'ndan eklenti kurabilirsiniz.
- **Gizli pencere** kapanınca hiçbir şey bırakmaz.
- **Otomatik güncelleme** (GitHub Releases üzerinden).

### Kurulum

[Releases sayfasından](https://github.com/PEEPWICK/peepwick-browser/releases) AppImage ya da
.deb dosyasını indirin. İkisi de kendini günceller: tarayıcı açılışta ve dört saatte bir yeni
sürümü denetler, arka planda indirir ve yeniden başlatınca kurar.

### Sürüm yayınlama

```bash
npm run release -- patch
git push --follow-tags
```

Etiket gönderildiğinde **Release** iş akışı AppImage ve .deb paketlerini derleyip GitHub'da
yayınlar. Kurulu tarayıcılar yeni sürümü dört saat içinde, ya da *Ayarlar › Hakkında*
bölümünden hemen alır.

### Lisans

GPL-3.0. PEEPWICK adı ve Çift Göz W işareti GPL kapsamında değildir; değiştirilmiş bir sürüm
dağıtırsanız kendi ad ve simgenizi kullanın.
