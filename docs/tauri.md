# Tauri + Rust geçişi

PEEPWICK Browser'ın Electron altyapısından Tauri 2 + Rust'a taşınması. Bu belge kararları,
mimariyi ve yayın ölçütü olan özellik eşitliği listesini tutar; geçiş bitince kaldırılır.

## Kararlar (2026-10-31)

| Konu | Karar |
| --- | --- |
| Motor | WebKitGTK (webkit2gtk-4.1), Tauri 2 kararlı sürüm |
| Platform | Şimdilik yalnızca Linux (AppImage + deb) |
| Arayüz | React arayüzü (`src/renderer`) kalır; arka uç Rust (`src-tauri`) |
| Yer | Aynı repo. Geliştirme `tauri` dalında, yan klasörde (`../peepwick-browser-tauri`) başladı; 0.2.0a ile `main` oldu |
| Yayın | İlk karar: eşitlik listesi tamamlanınca 0.2.0a. 2026-11-01'de değişti: 0.2.0a eşitlikten önce, GitHub'da ön sürüm (Pre-release) olarak yayınlandı. `main` Tauri koduna geçti, Electron kodu `electron` dalında; 0.1.9a "Latest" kalıyor ki kurulu 0.1.x kopyaları eksik bir sürüme kendiliğinden geçmesin. `package.json`'daki `githubPrerelease` eşitlik sağlanınca `false` yapılır |
| Kullanıcı verisi | Temiz başlangıç: Electron profilinden otomatik aktarım yok. Kullanıcılar yer imlerini HTML, parolaları CSV olarak dışa aktarıp yeni sürümde içe alabilir |
| Hedefler | Daha az bellek, daha küçük kurulum, Rust'ın güvenliği, Electron'dan bağımsızlık |

## Mimari

- **Tauri:** uygulama yaşam döngüsü, pencere, React arayüzünün web görünümü, arayüzle Rust arasındaki
  komutlar ve olaylar, paketleme, güncelleme.
- **Sekmeler:** Rust'tan doğrudan yönetilen WebKitGTK görünümleri (`webkit2gtk` crate). Pencerede bir
  `gtk::Overlay` sekme görünümlerini ve arayüz görünümünü üst üste tutar; menü ya da iletişim kutusu
  açıkken arayüz üste alınır (Electron'daki "overlay" modelinin karşılığı). Tauri'nin çoklu web görünümü
  hâlâ `unstable` ve WebKit'in indirme, sertifika, içerik kuralı arayüzlerini açmadığı için kullanılmaz.
- **Sayfa betikleri:** WebKit kullanıcı betikleri (otomatik doldurma ve çerez bildirimi için yalıtılmış
  dünyada, parmak izi koruması için sayfanın dünyasında) ve betik mesaj işleyicileri.
- **Ağ kancaları:** WebKit'in sayfa işlemine yüklenen küçük bir eklenti (`src-tauri/webext/peepwick-webext.c`;
  `build.rs` derler, paketlere kaynak dosyası olarak girer). Şimdilik her HTTP isteğine Global Privacy
  Control başlığını ekliyor: ayar her sayfa işlemine açılırken verilir, değişince mesajla bildirilir. Bu
  kadar küçük bir parça için Rust bağlaması gerekmediğinden C ile yazıldı; kalkanın sayfa başına
  sayaçlarını üstlenecek kadar büyürse Rust'a (cdylib) taşınabilir.
- **Kalkan:** Brave'in adblock-rust motoru (MPL-2.0). Ağ kuralları WebKit içerik kurallarına
  dönüştürülür, kozmetik kurallar betik ve stil olarak eklenir.
- **Veri:** geçmiş ve site simgeleri SQLite (rusqlite), ayarlar ve listeler JSON, parolalar sistem
  anahtarlığıyla (Secret Service) şifreli.

## Gidenler

- Chrome eklentileri ve Eklentiler sayfası (WebKitGTK'da karşılığı yok).
- DRM / Widevine: Netflix, Spotify gibi korumalı videolar oynamaz.
- Resim içinde resim: WebKitGTK'nın bunu uygulamalara açtığı doğrulanmadı; prototipte kesinleşecek.
- Güvenli DNS (DoH): WebKitGTK'da ayar yok. Yerine sistem düzeyinde DNS-over-TLS yönlendirmesi.
- Electron profilinden aktarım (temiz başlangıç kararı).

## Aşamalar

0. **Ortam ve iskelet:** Rust, WebKitGTK geliştirme paketleri, Tauri iskeleti, React arayüzünün Tauri
   köprüsüne bağlanması.
1. **Riskli noktaların denemesi:** sekme görünümleri ve arayüzün üste alınması, içerik kurallarıyla
   engelleme, sayfa işlemi eklentisi, zor siteler (YouTube, Google Meet, bankalar), bellek ve paket
   boyutu ölçümü. Sonuçlar gösterilip onay alınır.
2. **Temel tarama:** aşağıdaki "Tarama" bölümü ve kalkan.
3. **Veri ve gizlilik:** "Veri" ve "Gizlilik" bölümleri.
4. **Araçlar:** "Araçlar" bölümü.
5. **Paketleme ve yayın:** CI, imzalı güncelleme, README ve site, 0.2.0a.

## Aşama 1 sonuçları (2026-10-31)

Ölçümler bu makinede: Zorin OS 18 (Ubuntu 24.04 tabanı), WebKitGTK 2.52.6, Intel Alder Lake, 15 GB RAM.
Electron tarafı 0.1.9a.

**Çalışanlar**

- Mimari çalışıyor. React arayüzü Tauri'de değişmeden çiziliyor; sekmeler WebKitGTK görünümü olarak
  doğru yere yerleşiyor. Arayüz menü için üste alındığında sayfanın üzerinde saydam duruyor; bu, XWayland
  altında gerçek pencere görüntüsüyle doğrulandı.
- Kalkan: EasyList ve EasyPrivacy toplam 129.000 kural. adblock-rust bunları 0,3 saniyede çeviriyor,
  WebKit 7–9 saniyede derleyip diske yazıyor. Sonraki açılışlarda 0,4 saniyede yükleniyor.
- YouTube oynuyor (sessizken kendiliğinden, kare düşmeden).
- İş Bankası, GitHub, Google sayfaları sorunsuz çiziliyor.

**Boyut**

| | Electron 0.1.9a | Tauri + WebKitGTK |
| --- | --- | --- |
| deb | 104 MB | 3,2 MB |
| AppImage | 131 MB | 81 MB |

**Bellek** (aynı 5 site: example.com, Vikipedi, GitHub, DuckDuckGo, YouTube; işlem ağacının PSS toplamı)

| | Electron 0.1.9a | Tauri + WebKitGTK |
| --- | --- | --- |
| Toplam | 850–955 MB | 1310–1540 MB |
| YouTube sekmesi | 345 MB | 575–616 MB |
| Diğer sekmeler | 24–74 MB | 43–160 MB |

WebKit'in bellek baskısı ayarları ve önbellek modeli sonucu değiştirmedi; fark sayfaların canlı
JavaScript verisinden geliyor.

**Eksikler**

- WebRTC yok: Ubuntu'nun WebKitGTK paketi WebRTC olmadan derlenmiş. Google Meet, Teams, Discord
  görüşmeleri çalışmaz.
- Resim içinde resim yok: WebKitGTK'da `requestPictureInPicture` tanımsız.
- DRM yok (beklenen).
- Kullanıcı aracısı `Version/60.5 Safari` bildiriyor; düzeltildi (`Version/26.0 Safari`).
- Çok dilli sayfalar yavaş: `www.wikipedia.org` giriş sayfası bu makinede 20–35 saniyede açılıyor, o sırada
  sayfa işlemi yüzde 100 işlemci kullanıyor. Tetikleyici, uzun bir yazı tipi listesi
  (`-apple-system, "Segoe UI", Roboto, Inter, Helvetica, Arial, sans-serif`) ile yüzlerce farklı `lang`
  özniteliğinin birleşimi: WebKitGTK (Skia) yazı tipi listesini her dil için baştan çözüyor ve yazı tipi
  dosyalarını yeniden açıyor. İkisinden biri kaldırılınca sayfa hemen açılıyor; betikler, donanım
  hızlandırma ve yazı tipi sayısı (2.944 ya da 22) sonucu değiştirmiyor. Electron'da sorun yok. WebKit'e
  bildirilebilir; bizim tarafta bir çözüm yok.

**Sistem sorunu**

`~/.cache/fontconfig` içindeki `*.cache-9 → *.cache-12` sembolik bağlantıları (27 Eylül'de bir uygulama
ya da betik bırakmış) WebKitGTK'nın sayfa işlemini açılışta sonsuz döngüye sokuyor. Temiz önbellekle
sorun yok. Electron etkilenmiyordu.

## Eşitlik listesi

Durum: ☐ yapılmadı · ◐ kısmen · ☑ tamam (Electron sürümüyle aynı davranış, testli).

### Tarama
- ◐ Dikey sekmeler, alanlar, sabitleme, sürükle-bırak, alanlar arası taşıma (Rust tarafı hazır; sürükle-bırak elle denenecek)
- ◐ Yan yana görünüm, kullanılmayan sekmeleri uyutma (yan yana görünüm testli; uyutma denenecek)
- ◐ Oturumu geri yükleme, kapatılan sekmeyi yeniden açma (geri yükleme testli, ileri-geri geçmişiyle; session.json sürüm 2)
- ◐ Adres çubuğu: öneriler (geçmiş, açık sekmeler, arama motorunun önerileri; yer imleri onlar taşınınca), arama motorları
- ◐ Komut çubuğu (Ctrl+K): komutlar ve açık sekmeler; geçmiş ve yer imleri onlar taşınınca
- ◐ Yeni sekme sayfası, karşılama ekranı (testli: karşılamada dil seçimi ve başlama, en çok ziyaret edilenler, sayfanın arama kutusu ve önerileri, Ctrl+T'de klavyenin bu kutuya geçmesi, orta tıklamayla yeni sekme). Alttaki satırdaki bugünkü odak dakikaları odak moduyla geldi; bu hafta engellenenler Kalkan sayaçlarını bekliyor
- ◐ Yer imleri: çubuk, yönetici, yıldız, klasör menüleri, HTML içe ve dışa aktarma (0.2.1a; bookmarks.json Electron'la aynı biçim). Başka tarayıcılardan aktarma "Veri" bölümünde
- ◐ Geçmiş sayfası, site başına silme, en çok ziyaret edilenler, site simgeleri (`peepwick://favicon/`); history.db Electron'la aynı şema. Aralıkla silme "Tarama verilerini temizleme"de
- ◐ İndirmeler: panel ve sayfa, yeniden deneme, klasörde gösterme, nereye kaydedileceğini sorma, bitince bildirim (0.2.1a). WebKitGTK duraklatıp sürdüremiyor; düğmeleri kaldırıldı, "yeniden dene" baştan indirir
- ◐ Sayfada bul, yazdırma, sayfayı kaydetme (MHTML), geliştirici araçları
- ◐ Yakınlaştırma: site başına, varsayılan yakınlaştırma, yazı boyutu
- ◐ Sağ tık menüleri, sekme menüsü, uygulama menüsü (arayüzde çizilen; testli). Eksik: "sözlüğe ekle" (WebKitGTK'de API yok), resim içinde resim; yer imi ve okuma listesi girdileri onlar taşınınca
- ◐ Site izinleri (konum, kamera, mikrofon, bildirimler) ve izin çubuğu (0.2.1a; permissions.json Electron'la aynı). Ekran paylaşımı WebRTC'siz WebKitGTK'da yok, pano okuma izni bu WebKit sürümünün bağlamasında yok; ikisi de reddedilir
- ☑ Site güvenliği göstergesi, kilit paneli (sertifika, izinler, çerez sayısı, site verisini silme), sertifika görüntüleyici, sertifika hatası ekranı ve "yine de devam et" (0.2.1a). WebKit kabul ettiği sertifikayı unutamıyor; "uyarıyı geri al" sayfa yüklenince uyarıyı yeniden gösterir
- ◐ Gizli pencere (diske yazmayan WebKit bağlamı; son gizli pencereyle silinir)
- ◐ Ayarlar (bölüm bölüm), tema, dil, kısayollar. Kaydetme, sıfırlama (0.2.2a; dil ve karşılama ekranı kalır) ve donanım hızlandırmasından sonra yeniden başlatma (0.2.2a) testli; odak bölümü odak moduyla çalışıyor; parola, otomatik doldurma ve çeviri bölümleri o özellikler taşınınca
- ☑ Odak modu: 5–240 dakikalık oturum (kenar çubuğundaki panel, uygulama menüsü, komut çubuğu), listedeki sitelerde kapı, 15 saniye sonra "yine de aç" ile 5 dakikalık geçiş, site bildirimlerini susturma, odak dakikaları; `focus.json` ve `stats.json` Electron biçiminde. Testli: adres yazınca istek gitmeden kapı, bağlantı tıklanınca kapı, sayfadaki çerçevenin (gömülü video) yine açılması, geri dönünce alttaki sayfa, yine de aç, süre dolunca bitiş ve dakikaların sayılması, yeniden başlatmada süren oturum, odaktayken bildirim izninin kaydedilmeden reddi. Fark: bağlantı tıklamalarında kapı yanıt gelince iner, çünkü WebKitGTK çerçeveyi ana sayfadan ancak orada ayırıyor; istek siteye gider ama sayfa gösterilmez
- ◐ Hata ekranları (bağlantı, çökme, sertifika hatası ve "yine de devam et")

### Gizlilik
- ◐ Kalkan: standart ve kapalı, site istisnaları ve paneldeki aç/kapat (0.2.1a; sekmenin içerik kuralları siteye göre takılıp çıkarılır, yeniden derleme yok). Kurallar sekmenin kendi sayfasının sitesine bakar, ana sayfanın yanıtı gelince ve sayfa açılırken ayarlanır; 0.2.2a'ya kadar gezinme kararı aşamasındaydı ve sayfadaki iframe'ler sekmenin Kalkan'ını açıp kapatabiliyordu (düzeltme testli: izinli sitedeki başka siteden iframe Kalkan'ı açmıyor, normal sitedeki izinli siteden iframe kapatmıyor). Sıkı düzey ve sayaçlar yok: WebKitGTK engellenenleri bildirmiyor; bunu sayfa işlemi eklentisinin üstlenmesi gerekir
- ◐ Üçüncü taraf çerezleri engelleme (WebKit politikası ve ITP)
- ☑ Global Privacy Control (0.2.2a): `Sec-GPC: 1` her HTTP isteğinde (sayfa, alt kaynaklar, çerçeveler; sayfa işlemi eklentisinden) ve `navigator.globalPrivacyControl` sayfanın dünyasında, bütün çerçevelerde. Electron yalnızca başlığı gönderiyordu; çerez onay araçları çoğunlukla betikteki değeri okuduğu için ikisi birden
- ☑ HTTPS-Only modu (0.2.1a; üst düzey gezinmeler https'e yükseltilir, yanıt yoksa ya da başarısızsa uyarı, "HTTP ile devam et" oturum boyunca). Yükseltme artık sayfa işlemi eklentisinde, istek gitmeden önce ve yalnızca sekmenin kendi sayfası için: WebKitGTK'nın gezinme kararı ana sayfayı iframe'den ayıramıyor, 0.2.2a'ya kadar bir HTTP iframe'i ana sayfayı kendi HTTPS adresine götürebiliyordu. Eklenti ana sayfa isteğini `Sec-Fetch-Dest: document` başlığından, bu başlığın gelmediği güvenilmez adreslerde sayfanın adresiyle karşılaştırarak tanır; HTTPS sürümünün sayfayı HTTP'ye geri göndermesini de orada yakalar. Testli: iframe'ler HTTP kalır ve sayfa yerinde durur, yükseltme ve başarısızlıkta uyarı, döngüde uyarı, "HTTP ile devam et" ve izinli sitenin yeni sekmede de HTTP kalması
- ☑ Zararlı site uyarısı (`safebrowsing.rs`): malware-filter listeleri (URLhaus; OpenPhish, PhishTank, IPThreat), profilde 0.1.x'teki adlarla (`safebrowsing-malware.txt`, `safebrowsing-phishing.txt`), 12 saatte bir indirilir, saatte bir bakılır; ayar açılınca eksik liste hemen gelir. Listeler açılışta, sayfalar açılmadan diskten okunur (yaklaşık 36 bin alan adı, hata ayıklama derlemesinde 37 ms), böylece oturumdan geri gelen sekmeler de denetlenir. Alan adı ve üstündeki alan adları aranır; IP adresleri yalnızca bütün olarak, çıplak üst düzey alan adı hiç. Uyarı odak kapısıyla aynı iki yerde iner: adres yazınca istek gitmeden, bağlantı, yönlendirme ve geri-ileri yanıt aşamasında (indirmeden önce, listedeki sitenin dosyası kaydedilmez). Çerçevelere bakılmaz (0.1.x'teki gibi). "Yine de devam et" siteyi tarayıcı kapanana kadar açar. Fark: "Güvenli sayfaya dön" alttaki sayfayı gösterir (Electron geçmişte bir adım geri gidiyordu, bağlantının tıklandığı sayfayı da atlıyordu); bağlantılarda istek siteye gider ama sayfa gösterilmez. Testli: gerçek listelerin indirilmesi (18.470 + 18.138 alan adı), adres yazınca istek gitmeden uyarı (kimlik avı; alt alan adında kötü amaçlı yazılım), sayfadaki listeli çerçevenin yüklenmesi, bağlantı tıklanınca uyarı, geri dönünce alttaki sayfa, listeli siteden indirme bağlantısında dosya kaydedilmemesi, devam edince sayfanın ve yeni sekmede aynı sitenin açılması, ayar kapalıyken uyarı olmaması
- ☑ Parmak izi koruması (0.2.1a; sayfanın dünyasında WebKit kullanıcı betiği, tohum oturum anahtarı + üst sitenin adı)
- ☑ Çerez bildirimlerini reddetme (0.2.1a; ayrı betik dünyasında, bütün çerçevelerde)
- ☑ Tarama verilerini temizleme (0.2.2a; Ctrl+Shift+Delete; `cleardata.rs`). Geçmiş ve indirmeler aralıkla; çerezler ve site verileri, önbellek ve izinler Electron'daki gibi tüm zamanlar için (WebKit çerezleri yaşına göre silemiyor; HSTS listesi kalır). Çerez sayısı yerine veri tutan site sayısı gösterilir. Tüm geçmiş silinince yer imli sitelerin simgeleri kalır, WebKit'in sayfa simgesi deposu boşalır. Site başına silme kilit panelinde ve geçmiş sayfasında. Kapanışta temizleme Ctrl+Q'yu, son pencerenin kapanmasını ve yeniden başlatmayı bekletir (pencereler gizli, en çok 4 sn); düzgün kapanmayan bir çalışmanın (çökme, bilgisayarı kapatma) bıraktığı, sonraki açılışta sayfalar açılmadan silinir. Form verisi ve parolalar o özelliklerle gelir (şimdilik 0). WebKit önbelleği geliştirme sürümünde `com.peepwick.browser.dev`, testte profilin `cache` klasöründe
- ☑ Arayüz kendi sayfasından ayrılmaz, pencere açmaz: bağlantı, bırakılan adres ya da `window.open` onu başka bir sayfaya götüremez (Electron'daki `will-navigate` ve `setWindowOpenHandler` karşılığı; `window.rs` `on_navigation`, `on_new_window`). Adres değişikliği, `window.open` ve iptal edilemeyen orta tıklamayla testli; bırakılan adres aynı yoldan geçer, elle denenmedi

### Veri
- ☐ Parola yöneticisi: kaydetme, güncelleme, parolalar sayfası, zayıf ve tekrar eden denetimi, CSV
- ☐ Otomatik doldurma: giriş, kayıt formunda güçlü parola, adresler, form girdileri
- ◐ Başka tarayıcılardan aktarma (`importers.rs`): yer imleri ve geçmiş. Chromium ailesi (Chrome, Chrome Beta ve Dev, Chromium, Brave, Edge, Vivaldi, Opera) ve Firefox ailesi (Firefox, LibreWolf, Zen, Floorp), snap ve Flatpak kurulumlarıyla; profil adları Chromium'da `Local State`, Firefox'ta `profiles.ini`'den. Aynı klasöre giden iki yol bir kez sayılır, profilde yalnızca bulunan türler sunulur. Veritabanları tarayıcı açıkken de okunur: `-wal`/`-shm` dosyalarıyla yalnızca bizim okuyabildiğimiz geçici bir klasöre kopyalanır, sonra silinir. Okuma ana iş parçacığının dışında; yer imleri HTML aktarmasıyla aynı yoldan eklenir (çubuk boşsa doğrudan, doluysa "İçe aktarılanlar · Tarayıcı" klasörüne; mobil yer imleri ve Firefox'un menü klasörü diğer yer imlerine), yer imli sitelerin simgeleri de gelir (32 piksele en yakın boyut, en çok 100 KB). Geçmişten son 100 bin ziyaret, 5000'lik işlemlerle; aynı adres ve zamandaki ziyaret ikinci kez yazılmaz. Gizli pencerede aktarılmaz. Testli (sahte HOME ile Brave'in iki profili ve Firefox): profillerin bulunması, yer imleri, tekrarların atlanması, yalnızca geçmiş, simgeler ve geçmiş sayfasında görünmeleri. Parolalar, çerezler ve form verileri o özelliklerle gelir
- ☐ Şifreli yedek (PWBK biçimi, scrypt + AES-256-GCM) ve geri yükleme

### Araçlar
- ☐ Okuma modu, okuma listesi (çevrimdışı kopya), sesli okuma
- ☐ Sayfa çevirisi (Bergamot)
- ☐ Görev yöneticisi (Shift+Esc), medya denetimleri
- ☐ Hata bildir, hata günlüğü
- ☑ Varsayılan tarayıcı olma, otomatik güncelleme. Varsayılan tarayıcı (0.2.2a): deb'in "PEEPWICK Browser.desktop" dosyası; AppImage, 0.1.x gibi `~/.local/share/applications/peepwick-browser.desktop` dosyasını (ve simgesini) tutar, açılışta kendi yoluna çeker (0.1.x'ten kalan dosya da). xdg-settings ve xdg-mime arka planda, zaman sınırıyla çalışır; geliştirme sürümü "development" döner (testte `PEEPWICK_TEST_DEFAULT_BROWSER=1`, sahte XDG klasörleriyle). Güncelleme 0.2.1a'da: Tauri updater, minisign imzası (sürüm imzada, `requireSignedVersion`), liste `updates` dalında `latest.json`, AppImage hemen, deb yeniden başlatınca pkexec ile
- ◐ Türkçe ve İngilizce arayüz (Rust tarafı aynı sözlükleri derlemede gömer)
