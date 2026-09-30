# Erk Engine yol haritası

Erk gömülü bir HTML/CSS masaüstü UI motorudur: JavaScript yok, host uygulama
DOM'u sürer. Bu doküman kilometre taşlarının kapsamını ve kabul kriterini
tanımlar. Yön değişikliğinin gerekçesi [p1-embedded.md](../design/p1-embedded.md),
render hattının ve DOM modelinin ayrıntısı
[p0-architecture.md](../design/p0-architecture.md).

**Kural:** her kilometre taşı kendi başına çalışır durumda kalır ve
gösterilebilir bir çıktıyla biter — pencerede bir sayfa, bir PNG, bir taban
çizgisi JSON'u. Yarım kalmış bir taşın üzerine bir sonraki başlamaz.

**Süre tahmini yok.** Bu tek kişilik, AI destekli bir proje; zamanın büyük kısmı
kod yazmaya değil spesifikasyon okumaya ve kütüphaneler arası hata ayıklamaya
gidiyor. Ay tahmini bu gerçeği saklamaktan başka bir işe yaramaz.

---

## Durum

| KT | Kapsam | Durum |
|---|---|---|
| **M0** | İlk piksel | Bitti |
| **M0.5** | Mimari sözleşme | Bitti |
| **M1** | Statik UI | Yeni |
| **M2** | Etkileşim temeli | Yeni |
| **M3** | Kütüphane (Rust API, C-ABI) | Yeni |
| **M4** | Etkileşimli DOM | Yeni |
| **M5** | Artımlı render ve formlar | Yeni |
| **M6** | Python | Yeni |
| **M7** | Geliştirici araçları | Yeni |
| **M8** | Ürünleşme | Yeni |
| **M9** | Kompozitör ve performans | Yeni |

### Yön değişikliği (2026-09-30)

İlk rota tam bir masaüstü tarayıcıydı (M4'te JavaScript, M6'da Fetch ve ağ
güvenliği, M3'te kum havuzu). Rota gömülü bir UI motoruna çevrildi: tarayıcının
çok yıllık katmanlarının hiçbiri masaüstü UI için gerekmiyor, M0'da kurulan her
şey ise yeni hedefe doğrudan yarıyor. Gerekçe ve değerlendirme
[p1-embedded.md](../design/p1-embedded.md)'de.

### Neden önce piksel

İlk taslakta M0 süreç iskeleti, IPC ölçümü ve Windows kum havuzuydu. Sıra
tersine çevrildi: **önce çizdir, sonra kural koy.** Kabuk ile renderer M0'dan
itibaren yalnızca düz veri mesajlarla konuşuyor; bu disiplin şimdi C-ABI'nin
temeli.

---

## M0 — İlk piksel

Tek süreç, ağ yok, yerel dosya. Amaç yalnızca pikseli görmek.

- `erk <dosya.html>` yerel dosyayı açar
- html5ever 0.39 → arena DOM (`NodeId` = u32 indeks + u32 nesil)
- Stylo, paralellik kapalı; adaptör Blitz'ten uyarlanır ve Stylo onun
  derlendiği sürümle (0.20.x) başlar
- Taffy 0.14 block layout
- Paragraf, Taffy'de ölçüm fonksiyonlu bir yaprak: Parley şekillendirip
  satırlara böler, Taffy yalnızca `(genişlik, yükseklik)` bilir. **Tam IFC
  değil.**
- Erk display list (dikdörtgen + glyph run) → `vello_cpu`
- winit + softbuffer penceresi; renderer ayrı iş parçacığında, tipli mesajlarla
- `--screenshot out.png` başsız mod
- **Chrome referans testi:** aynı sayfalar Chrome'da ve Erk'te çizilip
  karşılaştırılır; sayfa başına içerik skoru kayıtlı beklentiye eşit kalır,
  yükselirse beklenti yükseltilir, yalnızca yazılı gerekçeyle düşer (kural
  CLAUDE.md'de)

Uygulama planı: [m0-first-pixel.md](m0-first-pixel.md).

**Kabul:** Türkçe başlık ve paragraf içeren biçimli yerel bir sayfa pencerede
görünüyor. Aynı sayfa `--screenshot` ile deterministik bir PNG veriyor ve bu PNG
bir altın dosya testiyle sabitleniyor.

---

## M0.5 — Mimari sözleşme

Kod değil belge; ama M1'in önkoşulu. M1 ve M2'nin Rust çekirdeği C-ABI'ye
sonradan uydurulmak zorunda kalmasın diye sınır şimdi çizilir.

- [p1-contract.md](../design/p1-contract.md) ve içinde kâğıt üzerinde bir
  `erk.h` taslağı:
  - `NodeId` opak `uint64_t`; 0 geçersiz; eski id → `ERK_ERR_STALE_NODE`
  - her çağrı bir durum kodu döner; sınırda panik yakalanır, uygulama
    zehirlenmiş sayılır, FFI'dan panik sızmaz
  - dizeler: girişte UTF-8 + uzunluk, Erk kopyalar; çıkışta çağıranın
    tamponu ya da `erk_string_free`
  - iş parçacığı modeli: API yalnızca UI iş parçacığından, başka iş
    parçacıkları için yalnızca `erk_app_post`; callback'ler UI iş
    parçacığında, layout ya da boyama sırasında asla
  - callback ömrü: `user_data` + isteğe bağlı `destroy`, tam bir kez
  - olay döngüsü: Erk'in döngüsü (`erk_app_run`) ve host'un döngüsüne gömülme
    (`erk_app_pump`, raw-window-handle)
  - kaynaklar host'un callback'inden, zaman host'un `now_ns`'inden,
    yapılandırma parametrelerden
  - sürümleme: `erk_abi_version()`, bir ana sürüm içinde yalnızca ekleme
- [css-support.md](../css-support.md): Supported / M1 / Later / Not planned
- Her sözleşme kuralı için hangi taşta hangi muhafızın geldiği

**Kabul:** Sözleşmenin her kuralının bir muhafız taşı var. CSS matrisi M1'in
kapsamını tanımlıyor ve "Not planned" listesi gerekçeli.

---

## M1 — Statik UI

Motorun özgün layout işi burada: **inline formatting context.** Taffy satır
kutularını bilmez; Parley metni şekillendirir ama kutuları satıra dizmez. İkisini
bağlayan katmanı Erk yazar. Kapsam masaüstü UI'ına göre daraltılmıştır: block,
inline metin, flex, absolute positioning. **Float, clear ve tablo düzeni yok**
(css-support.md "Not planned"); `float` hesaplanmış stilde kalsa da layout'ta
`none` sayılır ve metni düşürmez.

Layout tek PR'a sığmaz. M1 sırayla adımlara bölünür; her adım kendi PR'ı,
kendi testleri ve render değiştiriyorsa kendi Chrome referans sayfasıyla gelir.

| Adım | Kapsam | Durum |
|---|---|---|
| M1.0 | Ölçüm ve M0 eksikleri: yayın ikilisinin boyutu, 1000 düğümlü bir sayfada boştaki bellek, ilk kare süresi (bütçe buradan konur); blok ve metin karışık ebeveynde düşen metin (anonim kutular). Span stillerinin düzleşmesi IFC'nin kendisi olduğu için M1.3'te | Bitti |
| M1.1 | Tek satır metin: Parley ile şekillenen bir Taffy yaprağı | M0'da var (`a_paragraph_is_one_line_high`) |
| M1.2 | Satır kırma: daralan kutuda metin alt satıra iner | M0'da var (`narrow_width_breaks_into_more_lines`) |
| M1.3 | Tam IFC: inline kutular, `<span>`/`<b>`/`<i>` gibi farklı stillerin aynı satırda çizilmesi, satırlar arasında span kırılması, `text-align` (justify dahil), temel `vertical-align`, satır içi görseller. Blitz 0.3.0-beta.2 `layout/inline.rs` ve `construct.rs`'ten uyarlanır; calc değerleri Erk'in `CalcTable`'ından geçer; anonim blok kutularının yeri ilk iş olarak kararlaştırılır | Başladı: stil aralıkları var |
| M1.4 | Block ve absolute positioning doğrulaması (Taffy); float `none` gibi dizilir, metni düşürmez | Yeni |
| M1.5 | Flexbox doğrulaması (Taffy) | Yeni |
| M1.6 | Renk, kenarlık, yuvarlak köşe, gölge, `opacity`, görüntüler (png, jpeg); görüntüler ve CSS `url()` sözleşmenin kaynak API'sinden (demo kabukta bir kök dizin ve `memory://`) | Yeni |
| M1.7 | Sistem fontları ve fallback (fontique; gömülü font yalnızca testlerde), HiDPI cihaz ölçeği, `lang`'a göre `text-transform` (`icu_casemap`: Türkçede `i → İ`, `ı → I`) | Yeni |

Adımlar boyunca, ilk gerektiği adımda:

- WPT altyapısı: wptrunner'a `--screenshot` üzerinden koşan "erk" ürünü,
  yalnızca CSS dizinleri
- Sağlamlık: `render_html` hiçbir girdide paniklemez. `cargo test` içinde
  tohumlu bir girdi testi ve depodaki çökme korpusu; `fuzz/` altında
  cargo-fuzz, ayrı ve zaman sınırlı bir CI job'ı
- Lisans denetimi (`cargo deny check licenses`): ilk MPL-2.0 bağımlılıklar
  M0'da girdi; liste gömülü fontların OFL-1.1'ini de kapsar
- `erk-renderer` pencere katmanını bilmez (p1-contract §11)

Referanslar: Blitz `packages/blitz-dom/src/layout/inline.rs`, `construct.rs`
(0.3.0-beta.2, MIT OR Apache-2.0). Servo `layout` crate'i yalnızca okunur:
MPL-2.0, kopyalanmaz. Blitz'te `vertical-align` uygulaması yok; o iş Erk'in.

### Akış layout'u kararı

IFC önce Blitz modeliyle, Taffy'nin block layout'u içinde kurulur.
`css/CSS2/normal-flow` ve `css/css-text` için ilk taban çizgisi çıktığında kalan
testler "Taffy block kaynaklı", "IFC kaynaklı" ve "diğer" diye sınıflanır. Karar
bu sınıflandırmayla verilir ve bu dokümana yazılır: Taffy'de kalmak ya da akış
layout'unu Erk'e almak (Taffy yalnızca flex ve grid'de). Karar bir süreye
değil bu sonuca bağlıdır.

**Kabul:** Bir ayarlar ekranı maketi (form satırları, flex düzeni, kenarlıklar,
Türkçe metin) Chrome referans testinde skorlu. `css/CSS2/normal-flow`,
`css/css-flexbox`, `css/css-position`, `css/css-text` taban çizgileri JSON olarak
yayımlı ve gerileme yasağı CI'da. Fuzz job'ı yeşil. İkili boyutu CI'da bütçenin
altında; bellek ve ilk kare adlandırılmış bir makinede ölçülüp kaydedilmiş. Akış
layout'u kararı gerekçesiyle belgelenmiş.

---

## M2 — Etkileşim temeli

- GPU yolu: `vello_hybrid` (wgpu 29), yüzey oluşturulamazsa `vello_cpu`'ya
  düşme
- Girdi hattı: winit'in fare, klavye ve tekerlek olayları kabuktan renderer'a
  mesajla
- Hit-test: layout kutuları boyama sırasının tersinden gezilir
- Kaydırma kapları (`overflow: auto | scroll`), imleç biçimleri
- `:hover`, `:active`, `:focus` (Stylo'nun eleman durumu)
- Geliştirici araçlarının ilk parçası: `inspect_at(x, y)` (hit-test'in
  döndürdüğü `NodeId`) ve seçili düğümün kutusunu gösteren bir vurgu
  kaplaması (display list'e eklenen, belgeye ait olmayan bir öğe)
- macOS CI

**Bilerek kaba:** artımlı stil ve layout M5'te. M2'de her durum değişikliği
(hover, kaydırma) tam yeniden stil, layout ve boyama ister. M2'nin kare süresi
ölçümleri bir performans iddiası değil, M5'in kıyaslanacağı tabandır.

**Kabul:** Uzun bir sayfa kayıyor, hover stili değiştiriyor, bir tık doğru
`NodeId`'yi raporluyor (otomatik test). GPU yolu yoksa CPU'ya düşüyor. Tam
yeniden hesaplamanın kare süresi kaydedilmiş.

---

## M3 — Kütüphane

- `erk`: idiomatik Rust API'si (`App`, düğüm tutamakları, olay abonelikleri)
- `erk-ffi`: sözleşmedeki C-ABI; `erk.h` cbindgen ile üretilir. Adıyla
  listelenmiş `unsafe` istisnası (`#[unsafe(no_mangle)]`)
- İş parçacığı modeli, callback ömrü ve hata kodları sözleşmedeki gibi
- Kaynak sağlayıcı callback'i; demo kabuk `erk`'in ilk kullanıcısı olur
- Denetim sorguları (salt okunur, p1-contract §10.1): düğüm ağacı, etiket ve
  öznitelikler, hesaplanmış stil, kutu modeli (margin, border, padding,
  içerik). Kare başına aşama süreleri (stil, layout, display list, raster),
  çekirdeğin değil aşamaları çağıran `erk` crate'inin ölçümüyle
- Muhafızlar: üretilen `erk.h` depodakiyle aynı; bir C örneği CI'da derlenip
  çalışıyor; `unsafe` yalnızca `erk-style` ve `erk-ffi`'de

**Kabul:** Rust ve C örnek uygulamaları CI'da derlenip bir sayfa açıyor ve bir
tık olayı alıyor. Yanlış iş parçacığından çağrı ve eski `NodeId` hata kodu
döndürüyor (test). Her muhafız kasıtlı bir ihlalle denenmiş.

---

## M4 — Etkileşimli DOM

- Arenada silme: slot serbest listeye girer, nesil artar
- `Mutation` toplu API'si: oluştur, ekle, araya ekle, sil, metin, öznitelik,
  sınıf, satır içi stil
- Olay dağıtımı: DOM'un capture/bubble alt kümesi; tıklama, girdi, değişiklik,
  gönderim, klavye, odak
- `querySelector` ve `querySelectorAll` (selectors crate'i)
- `Mutation` dizileri fuzz'lanır (eski `NodeId`'ler dahil)
- İlk demo bir sayaç: Rust host ve HTML/CSS, düğmeye basıldıkça sayı artıyor.
  Tıklama olayını, metin değişikliğini ve yeniden çizimi uçtan uca bağlayan en
  küçük uygulama

**Kabul:** Sayaç demosu ve JS'siz, Rust host'lu bir TodoMVC çalışıyor. 10 bin oluştur/sil
döngüsünde bellek büyümüyor. Mutation fuzz'ı yeşil.

---

## M5 — Artımlı render ve formlar

- Kalıcı stil verisi ve Stylo'nun yeniden stil ipuçları; kalıcı layout yan
  tabloları ve Taffy önbelleği; kirlenme bitleri
- Form kontrolleri: `input` (metin, onay kutusu, radyo), `textarea`, `button`,
  `select`
- İmleç, seçim, pano; IME (Windows TSF ile Türkçe ve CJK)
- Odak ve Tab gezinmesi
- Erişilebilirlik: AccessKit ile DOM'un işletim sistemi erişilebilirlik ağacına
  çevrilmesi
- Canlı CSS düzenleme için temel: bir düğümün satır içi stilini ya da bir
  kuralı değiştirip artımlı yeniden stille görmek

**Kabul:** 10 bin düğümlü bir belgede bir metin alanına yazarken p95 kare süresi
hedefi (sayı bu taşın planında, M2 tabanına göre) tutuyor. Türkçe ve CJK IME
girişi çalışıyor. Bir ekran okuyucu form etiketlerini okuyor.

---

## M6 — Python

- `erk-python`: C-ABI üstünde cffi, maturin ile platform wheel'leri
- Nesne yönelimli sarmalayıcı (`App`, `Element`, `on("click", ...)`)
- Go ve diğer diller topluluğa açık; C başlığı ve örnekler yeterli

**Kabul:** `pip install erk` Windows ve Linux'ta çalışıyor; README'deki Python
örneği bir pencere açıp bir tıklamaya yanıt veriyor.

---

## M7 — Geliştirici araçları

M2, M3 ve M5'te gelen parçaların (vurgu ve `inspect_at`, denetim
sorguları, aşama süreleri, canlı stil) üzerine kurulan, **Erk ile yazılmış**
bir DevTools uygulaması. Motoru kendi geliştirici aracıyla sınar.

- Paneller: Elements (ağaç, öznitelikler), Styles (kurallar, `:hover` dahil,
  düzenlenebilir), Computed, Layout (kutu modeli), Events (host'a giden
  olaylar), Performance (kare başına aşama süreleri, zaman çizelgesi),
  Console, Resources, Accessibility
- **Console** JavaScript konsolu değildir: host'un `ErkLogFn` ile gönderdiği
  loglar ve Erk'in kendi uyarıları (engellenen kaynak, yüklenemeyen görüntü)
- **Resources** ağ paneli değildir: Erk'in host'tan istediği kaynaklar
  (`memory://`, dosyalar), türleri ve durumları. Host'un kendi ağ trafiği
  ancak host bunu olay olarak beslerse görünür
- F12 ile açılır; tuşu ve açılıp açılmayacağını host belirler, yayın
  derlemelerinde varsayılan kapalı
- Önce süreç içi, ikinci pencere olarak. Uzak DevTools (ayrı süreç, yerel
  bir kanal) ancak açıkça istenirse: motor varsayılan olarak hiçbir portu
  dinlemez
- HTML ve CSS için hot reload

**Kabul:** F12 ile açılan DevTools, kendi uygulamasında sayfadan bir öğe
seçiyor, stilini değiştiriyor, kutu modelini ve karenin aşama sürelerini
gösteriyor. DevTools'un kendisi de Erk ile çiziliyor.

---

## M8 — Ürünleşme

- `erk-cli`: `new`, `dev` (hot reload), `build`
- Varlık gömme: HTML, CSS ve görüntüler ikiliye gömülür, `memory://` üzerinden
  okunur
- Paketleme (Windows, Linux, macOS) ve kod imzalama rehberi
- C-ABI sürüm ve kararlılık politikası

**Kabul:** `erk build` dış dosyasız tek parça bir `.exe` ve AppImage üretiyor.

---

## M9 — Kompozitör ve performans

- CSS animasyonları ve geçişleri (zaman host'un `now_ns`'inden)
- Kaydırma katmanı başına tile cache, kompozitör iş parçacığında kaydırma

**Kabul:** Adlandırılmış bir sayfa kümesinde, adlandırılmış bir donanımda p95
kare süresi hedefleri (sayılar bu taşın planında) tutuyor.

---

## Bilerek kapsam dışı

| Ne | Neden |
|---|---|
| JavaScript ve her türlü betik | Motorun ilkesi; iş mantığı host'ta |
| Ağ, HTTP, Fetch, çerezler | Host'un işi; motor ağa hiç erişmez |
| Kum havuzu, çoklu süreç, site izolasyonu | İçerik host'un kendisi; mesajlar serileştirilebilir kaldığı için ihtiyaç olursa sonradan eklenebilir |
| Float, clear, tablo düzeni, multi-column, print/paged media | Masaüstü UI'ı flex ile kurulur; ayrıntı css-support.md'de |
| WebExtensions, medya ve DRM, WebRTC, WebXR | Tarayıcı işleri |
| Mobil platformlar | Hedef masaüstü |
| Kendi font ayrıştırıcı, shaper, görüntü çözücü | skrifa, HarfRust, png, jpeg çözücüleri var |
