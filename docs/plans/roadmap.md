# Erk Engine yol haritası

Erk gömülü bir HTML/CSS masaüstü UI motorudur: çekirdekte JavaScript yok
(isteğe bağlı bir bağlama olarak var), host uygulama DOM'u sürer. Hedef:
modern, açık kaynaklı, Rust ile güçlendirilmiş yeni nesil bir Sciter. Bu doküman kilometre taşlarının kapsamını ve kabul kriterini
tanımlar. Yön değişikliğinin gerekçesi [p1-embedded.md](../design/p1-embedded.md),
render hattının ve DOM modelinin ayrıntısı
[p0-architecture.md](../design/p0-architecture.md).

**Kural:** her kilometre taşı kendi başına çalışır durumda kalır ve
gösterilebilir bir çıktıyla biter — pencerede bir sayfa, bir PNG, bir taban
çizgisi JSON'u. Yarım kalmış bir taşın üzerine bir sonraki başlamaz.

**Süre tahmini yok.** Bu tek kişilik, AI destekli bir proje; zamanın büyük kısmı
kod yazmaya değil spesifikasyon okumaya ve kütüphaneler arası hata ayıklamaya
gidiyor. Ay tahmini bu gerçeği saklamaktan başka bir işe yaramaz.

**Karar zamanı.** Mimari tartışma yalnızca o anki taşı bloke ediyorsa
yapılır; etmiyorsa ilgili taşın planına açık soru olarak yazılır.

---

## Durum

| KT | Kapsam | Durum |
|---|---|---|
| **M0** | İlk piksel | Bitti |
| **M0.5** | Mimari sözleşme | Bitti |
| **M1** | Statik UI | Bitti |
| **M2** | Etkileşim temeli | Başladı: adım planı ([m2-interaction.md](m2-interaction.md)) |
| **M3** | Kütüphane (Rust API, C-ABI) | Yeni |
| **M4** | Etkileşimli DOM | Yeni |
| **M5** | Artımlı render ve formlar | Yeni |
| **M6** | Bağlamalar: Python, Go, JavaScript | Yeni |
| **M7** | Geliştirici araçları | Yeni |
| **M8** | Ürünleşme | Yeni |
| **M9** | Kompozitör ve performans | Yeni |
| **M10** | Surface: host'un GPU çizimi | Yeni |
| **M11** | SVG ve medya | Yeni |
| **M12** | Bileşenler ve ekosistem | Yeni |

### Yön değişikliği (2026-09-30)

İlk rota tam bir masaüstü tarayıcıydı (M4'te JavaScript, M6'da Fetch ve ağ
güvenliği, M3'te kum havuzu). Rota gömülü bir UI motoruna çevrildi: tarayıcının
çok yıllık katmanlarının hiçbiri masaüstü UI için gerekmiyor, M0'da kurulan her
şey ise yeni hedefe doğrudan yarıyor. Gerekçe ve değerlendirme
[p1-embedded.md](../design/p1-embedded.md)'de.

### JavaScript kararı (2026-10-01)

Çekirdekte JavaScript yok; JS, Python ve Go gibi isteğe bağlı bir bağlama
olarak M6'da geliyor (`erk-script`). Sayaç demosu M4'ten M2'nin kabulüne
çekildi. Gerekçe ve dış eleştirinin değerlendirmesi
[p1-embedded.md](../design/p1-embedded.md) §3.1'de.

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
| M1.3 | Tam IFC: inline kutular, `<span>`/`<b>`/`<i>` gibi farklı stillerin aynı satırda çizilmesi, satırlar arasında span kırılması, `text-align` (justify dahil), temel `vertical-align`, satır içi görseller. Blitz 0.3.0-beta.2 `layout/inline.rs` ve `construct.rs`'ten uyarlanır; calc değerleri Erk'in `CalcTable`'ından geçer; anonim blok kutularının yeri ilk iş olarak kararlaştırılır | Bitti: stil aralıkları, `text-align`, sağlamlık, satır içi kutular, `inline-block`, `vertical-align`. Satır içi görseller `<img>` ile M1.6'da; cargo-fuzz job'ı M1 bitmeden ayrı PR'da |
| M1.4 | Block ve absolute positioning doğrulaması (Taffy); float `none` gibi dizilir, metni düşürmez | Bitti: konumlandırma, `z-index`, float, WPT altyapısı (normal-flow %42,8, css-position %16,3), akış layout'u kararı (geçici: Taffy) |
| M1.5 | Flexbox doğrulaması (Taffy) | Bitti: `order`, flex kapsayıcıda metin, absolute elemanların statik konumu; `css/css-flexbox` %53,2 |
| M1.6 | Renk, kenarlık, yuvarlak köşe, gölge, `opacity`, görüntüler (png, jpeg); görüntüler ve CSS `url()` sözleşmenin kaynak API'sinden (demo kabukta bir kök dizin ve `memory://`) | Bitti: kenarlık, yuvarlak köşe, gölge, `opacity`, `<img>` ve `background-image` (png, jpeg) kaynak API'siyle, lisans kapısı (`cargo deny`) |
| M1.7 | Sistem fontları ve fallback (fontique; gömülü font yalnızca testlerde), HiDPI cihaz ölçeği, `lang`'a göre `text-transform` (`icu_casemap`: Türkçede `i → İ`, `ı → I`) | Bitti: `text-transform`, HiDPI, sistem fontları (host'ta fontique), `font-family`, yazı sistemine ve dile göre yedek, renkli emoji |

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

**Karar (2026-10-01, geçici): Taffy'de kalınır.** `css/CSS2/normal-flow`'un ilk
taban çizgisinde (WPT `5cd8e3f`) 746 reftest'ten 319'u geçiyor. Düşen 427
testin her biri Erk'in bugün desteklemediği ya da hiç planlamadığı bir şeyi
kullanıyor: dış stil sayfası ya da betik (75), görüntü ya da `url()` (230),
tablo (33), betik (41), float (8), kenarlık boyama (13), liste (14), üretilmiş
içerik (8) ve birkaç başka. Bunların hiçbirine dokunmayan ve yine de düşen
test **yok**: düşüşü Taffy'nin block layout'una bağlayan bir kanıt yok.
Sınıflama "bu özelliği kullanıyor" der, "bu yüzden düşüyor" demez; bu yüzden
karar geçicidir. M1.6'dan (görüntüler, kenarlıklar, kaynak API'si) sonra
sınıflama tekrarlanır, en büyük küme o zaman ölçülebilir hale gelir.
`css/css-text` taban çizgisi ayrı bir PR'da gelir (M1 kabulünün parçası).

**Karar (2026-10-02, kesin): Taffy'de kalınır.** Sınıflama M1.6'dan sonra
tekrarlandı. Görüntüler ve kenarlıklar gelince normal-flow 508/746 geçiyor
(tekrar sırasında bulunan bir `<img>` boyut hatası 86 testi tek başına
geçirdi). Düşen 238 testin 205'i desteklenmeyen ya da planlanmayan bir şeyi
kullanıyor (dış stil sayfası, tablo, betik, Erk'in çizmediği yerine konan
elemanlar, float); kalan 33'ün çoğu Erk'in kendi inline layout'unda (inline
içinde blok, 11) ya da tek tek boyut durumlarında. Taffy'nin block
layout'unu bırakmayı gerektiren bir küme yok. `css/css-text` taban
çizgisi: 608/1489.

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
- **Sayaç demosu:** düğmeye basılır, demo host'un (Rust) sayacı artar,
  sayının metni değişir, kare yeniden çizilir. Bunun için M4'ün `Mutation`
  API'sinin ilk parçası M2'de gelir: bir düğümün metnini değiştirmek.
  Tıklama, hit-test'in bulduğu düğümle host'a mesaj olarak döner; mesajlar
  düz veri kuralında kalır
- macOS CI

M2 de adımlara bölünür; her adım kendi PR'ı ([m2-interaction.md](m2-interaction.md)):

| Adım | Kapsam | Durum |
|---|---|---|
| M2.0 | Kalıcı belge (bir kez ayrıştırılır, kareler arasında yaşar), metin geometrisi testi (M1'in bulgusu), kare süresi tabanı | Bitti: 163 metin satırının 159'u Chrome'la 1 px içinde, iki gerçek hata bulundu; tam kare 1000 elemanda ~71 ms |
| M2.1 | Girdi mesajları, hit-test, tıklama olayı ve yayılma yolu, `inspect_at` ve vurgu kaplaması | Bitti: hit-test boyama sırasıyla, tıklama ortak ataya yoluyla, vurgu belgeye girmiyor; fare girdisi kare çizmiyor |
| M2.2 | `:hover`, `:active`, `:focus`; odak ve klavyeyle gezinme | Bitti: durum seçicileri, Tab sırası, Enter ve Space; durumu kullanmayan sayfa fare hareketinde yeniden çizilmiyor |
| M2.3 | `overflow` kırpması, kaydırma kapları ve tekerlek, `cursor` | Bitti: kırpma Chrome'la aynı (`overflow` sayfası), tekerlek iç içe kaplarda zincirleniyor; WPT'de 17 test geçmeye başladı |
| M2.4 | `SetText` ve `Query` (ilk `Mutation`), sayaç demosu ve altın görüntülü testi | Bitti: CSS seçicili sorgu, eski id hata; yükleme aynı arenaya (sözleşmeye aykırılık düzeltildi); sayaç altın görüntüyle |
| M2.5 | GPU yolu (`vello_hybrid`), CPU'ya düşme, ölçüm; host'a çizim için render hedefi (host'un penceresi) | Yeni |
| M2.6 | Metin düzenini sağlamlaştırma: inline-boxes, paragraphs, vertical-align farkları, satır içi kutu parçalanması, `white-space` kararı | Yeni |
| M2.7 | `css/css-position` analizi: düşen her test sınıflanır, desteklenen özelliklerdeki hatalar düzeltilir | Yeni |
| M2.8 | macOS CI, kabul | Yeni |

**Bilerek kaba:** artımlı stil ve layout M5'te. M2'de her durum değişikliği
(hover, kaydırma) tam yeniden stil, layout ve boyama ister. M2'nin kare süresi
ölçümleri bir performans iddiası değil, M5'in kıyaslanacağı tabandır.

**Kabul:** Uzun bir sayfa kayıyor, hover stili değiştiriyor, bir tık doğru
`NodeId`'yi raporluyor (otomatik test). Sayaç demosu çalışıyor: otomatik bir
test tıklama gönderip sayının değiştiği kareyi altın görüntüyle doğruluyor.
GPU yolu yoksa CPU'ya düşüyor. Tam
yeniden hesaplamanın kare süresi kaydedilmiş.

---

## M3 — Kütüphane

- `erk`: idiomatik Rust API'si (`App`, düğüm tutamakları, olay abonelikleri)
- `erk-ffi`: sözleşmedeki C-ABI, paylaşımlı kütüphane olarak (`erk.dll`,
  `liberk.so`, `liberk.dylib`); `erk.h` cbindgen ile üretilir. Adıyla
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
- **Tasarımcının ilk aradıkları** (2026-10-05 kararı, "Later"dan öne
  alındı): `linear-gradient` ve `radial-gradient` (vello'nun gradyan
  fırçaları; display list'te gradyan öğesi, Chrome referans sayfasıyla) ve 2D
  `transform` (`translate`, `scale`, `rotate`; boyama dönüşümü, hit-test
  tersine dönüşümle). Gerekçe: modern düğme ve kartlar düz renk yerine hafif
  gradyan kullanıyor; `:active`'te `scale(0.98)`, `:hover`'da birkaç piksel
  kayma en temel "dokunma hissi"
- `Mutation` dizileri fuzz'lanır (eski `NodeId`'ler dahil)
- M2'nin sayaç demosu genel API'ye taşınır; TodoMVC eleman oluşturmayı,
  silmeyi ve listeyi uçtan uca sınar

**Kabul:** Rust host'lu bir TodoMVC çalışıyor. 10 bin oluştur/sil döngüsünde
bellek büyümüyor. Mutation fuzz'ı yeşil.

---

## M5 — Artımlı render ve formlar

Mimari: [p2-incremental.md](../design/p2-incremental.md), Erk Invalidation
Core (EIC), nihai. Adımlar M5.0–M5.8; ilk adım ölçüm altyapısı, taban M2'nin
tam yeniden hesabı. Kirlenme `erk-invalidation` crate'inde, yalnızca `erk-dom`'a
bağımlı; düğüme bağlı veri yan tablolarda.

- Mutation journal: kare içi birikim, birleştirme, iç içe transaction (M4'ün
  `Mutation` API'si üstüne)
- Tek invalidation sözlüğü (stil, metin, layout, boyama, erişilebilirlik)
  ve her kirlenmenin nedeni; nedenler M7'nin DevTools'unda görünür
- Kalıcı stil verisi; seçici invalidation Stylo'nun (snapshot'lar, yeniden
  stil ipuçları, `:has()`), stil hasarı Erk'in bitlerine çevrilir
- Kalıcı layout yan tabloları ve Taffy önbelleği; hesaplanmış stilden
  yeniden yerleşim sınırları (`contain: size layout`, sabit boyut) ve erken
  kesme (çıktısı değişmeyen kutu yayılmayı durdurur)
- Kalıcı metin şekillendirmesi; kutu başına display list parçaları, hasar
  bölgesi, arka uçtan bağımsız `RenderBackend`, kısmi sunum
- Her artımlı yol tam yeniden hesapla karşılaştırılır: aynı mutasyon dizisi
  iki yoldan aynı display list'i vermek zorunda (M4 fuzz'ına bağlı)
- Form kontrolleri: `input` (metin, onay kutusu, radyo), `textarea`, `button`,
  `select`
- İmleç, seçim, pano; IME (Windows TSF ile Türkçe ve CJK)
- Odak ve Tab gezinmesi
- Erişilebilirlik: AccessKit ile DOM'un işletim sistemi erişilebilirlik ağacına
  çevrilmesi; ağaç yardımcı teknoloji etkinleşince kurulur, sonra yalnızca
  kirli düğümler gönderilir
- Davranışı olan standart elemanlar: `<details>`/`<summary>`, `<dialog>`,
  `popover` özniteliği, `commandfor`/`command`. Açılır menü, akordeon ve
  diyalog betiksiz ve host'a gitmeden çalışır; her biri Chrome referans
  sayfasıyla
- Canlı CSS düzenleme için temel: bir düğümün satır içi stilini ya da bir
  kuralı değiştirip artımlı yeniden stille görmek
- **Temel geçişler** (2026-10-05 kararı, M9'dan öne alındı): `transition`
  ile `color`, `background-color`, `opacity` ve `transform` için doğrusal
  enterpolasyon (lerp) ve standart zamanlama eğrileri; zaman host'un
  `now_ns`'inden (p1-contract §7). Rengin "çat" diye değil 150 ms'de
  yumuşakça değişmesi uygulamanın kalitesini belirliyor; tam animasyon
  motoru (`@keyframes`, kompozitörde koşan animasyonlar) M9'da kalır

**Kabul:** 10 bin düğümlü bir belgede bir metin alanına yazarken p95 kare süresi
hedefi (sayı bu taşın planında, M2 tabanına göre) tutuyor. p2-incremental §4'ün
B1–B11 ölçümleri tabana karşı yayımlı. Kısmi kare ile tam kare piksel piksel
aynı. Türkçe ve CJK IME girişi çalışıyor. Bir ekran okuyucu form etiketlerini
okuyor.

---

## M6 — Bağlamalar: Python, Go, JavaScript

Sıra: önce Python (ilk bağlama kararı), sonra Go, sonra JavaScript.

- `erk-python`: C-ABI üstünde cffi, maturin ile platform wheel'leri
- Nesne yönelimli sarmalayıcı (`App`, `Element`, `on("click", ...)`)
- `erk-go`: C-ABI üstünde cgo sarmalayıcısı. Seçici tabanlı kolaylıklar
  (`OnClick("send", ...)`) düşük seviye `NodeId` API'sinin üstünde durur,
  onun yerini almaz
- `erk-script`: isteğe bağlı JavaScript bağlaması
  ([p1-embedded.md](../design/p1-embedded.md) §3.1). C-ABI'nin değil `erk`'in
  Rust API'sinin üstünde, gömülü bir JS motoruyla. JS tarafı düğümlere
  yalnızca `NodeId` ile başvurur, DOM JS nesnesi tutmaz. DOM API'sinin küçük
  bir alt kümesi (seçiciler, metin, öznitelikler, `classList`, `style`,
  oluştur/ekle/sil, `addEventListener`); Web API'si yok; zamanlayıcılar
  host'un saatiyle. Varsayılan kapalı: Cargo özelliği açılmazsa ikiliye JS
  motoru girmez
- JS motoru ölçülerek seçilir: Boa (saf Rust) ile QuickJS (`rquickjs`, C)
  ikiliye eklediği boyut, açılış süresi ve TodoMVC'nin 10 bin işlemlik
  süresiyle karşılaştırılır; sonuç bu taşın planına yazılır. QuickJS
  seçilirse yeni bir C bağımlılığı olduğu için `docs/design/` altında ayrı
  bir karar belgesi gerekir
- Muhafızlar `erk-script`'le aynı PR'da: çekirdek crate'ler, `erk` ve
  `erk-ffi` hiçbir JS motoruna bağımlı değil (`cargo tree`); `erk-script`
  projeden yalnızca `erk`'e bağımlı. Boyut bütçesi özelliksiz ikiliyi
  ölçmeye devam eder; betikli derleme kendi bütçe satırını alır
- Diğer diller topluluğa açık; C başlığı ve örnekler yeterli

**Kabul:** `pip install erk` Windows ve Linux'ta çalışıyor; README'deki Python
örneği bir pencere açıp bir tıklamaya yanıt veriyor. Aynı örnek Go ile de
çalışıyor. Sayaç ve TodoMVC JavaScript ile yazılmış halde aynı host
kabuğunda çalışıyor; silinmiş bir düğüme dokunan betik istisna alıyor,
süreç çökmüyor. JS özelliği kapalı derlemenin bağımlılık ağacında JS motoru
yok. Python örneğinin penceresinin özel belleği M1.0'daki yöntemle ölçülüp
yayımlanmış; yorumlayıcının kendi payı ayrı yazılmış.

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
- C-ABI sürüm ve kararlılık politikası (ABI 1.0)
- `erk new` şablonları işletim sistemi entegrasyonunu **host kodu** olarak
  getirir: menü, sistem tepsisi, dosya diyalogları, bildirimler, genel kısayol
  tuşları, otomatik güncelleme (Rust'ta `muda`, `tray-icon`, `rfd` gibi
  crate'lerle). Bunların hiçbiri çekirdeğe girmez

**Kabul:** `erk build` dış dosyasız tek parça bir `.exe` ve AppImage üretiyor.
Gerçek bir uygulama kıyası: Nexus Mail'in bir ekranı hem Wails + React hem
Go + Erk ile yazılmış (gelen kutusu ve okuma paneli; yazma penceresi
kapsam dışı, p1-embedded §4); ikili boyutu, bellek ve açılış süresi aynı makinede
ölçülüp yayımlanmış.

---

## M9 — Kompozitör ve performans

- CSS animasyonları (`@keyframes`) ve kompozitörde koşan geçişler (zaman
  host'un `now_ns`'inden); temel geçişler M5'te
- **`backdrop-filter: blur`** (buzlu cam; 2026-10-05 kararı, "Not planned"dan
  alındı): kenar çubuğu ve diyalog arkalarında çok yaygın. Arkadaki içeriği
  bulanıklaştırmak bir kompozitör katmanı istiyor; bu yüzden burada, katman
  ağacıyla birlikte
- Kaydırma katmanı başına tile cache, kompozitör iş parçacığında kaydırma
- Ölçüm kapısı: bağımsız alt ağaçların layout'u ve erişilebilirlik eşitlemesi
  için bir iş grafiği (p2-incremental §3.9), ancak M5'in ölçümleri tek iş
  parçacıklı yolun kare bütçesini aştığını gösterirse

**Karar (2026-10-05):** temel geçişler (renk, opaklık, `transform`) M5'e
alındı; M9'da `@keyframes` animasyonları ve kompozitör iş parçacığında koşan
geçişler kalır.

**Kabul:** Adlandırılmış bir sayfa kümesinde, adlandırılmış bir donanımda p95
kare süresi hedefleri (sayılar bu taşın planında) tutuyor.

---

## M10 — Surface: host'un GPU çizimi

`<canvas>` ve WebGL JavaScript'e bağlı; Erk'teki karşılığı, host'un belgenin
içindeki bir bölgeye kendi GPU çizimini yapabilmesi (grafikler, harita,
3B görünüm).

- Host bir düğümü surface olarak kaydeder (`erk_surface_create(node)`);
  işaretleme için HTML özniteliği değil, API kullanılır
- **Çizim sahipliği tek elde:** aynı pencere yüzeyine iki ayrı çizici
  yazmaz. Erk kareyi oluştururken surface bölgesi için host'un çizim
  callback'ini çağırır (paylaşılan wgpu cihazı, host'un dokusu) ya da
  host'un verdiği dokuyu bölgeye yerleştirir. Seçim M2'nin GPU yolu ve M3'ün
  API'si oturduktan sonra ölçülerek yapılır; sözleşmedeki taslak p1-contract
  §10.2
- Bölgenin konumu ve boyutu layout'tan gelir, kaydırma ve kırpma Erk'indir

**Kabul:** Bir uygulama, Erk ile çizilen araç çubuğu ve kenar çubuğunun
arasında wgpu ile kendi çizdiği bir grafiği gösteriyor; pencere
boyutlanınca ve sayfa kayınca grafik doğru yerde kalıyor.

---

## M11 — SVG ve medya

- SVG: resvg ile ayrıştırma, çizim vello'ya (resvg'nin çizim ağacı üzerinden)
- `<video>`: işletim sisteminin çözücüleriyle, kareler bir surface dokusuna
- Ses Erk'in değil host'un işi

**Kabul:** Simge seti SVG olan bir ekran doğru çiziliyor (Chrome referans
testi); bir video bir surface içinde oynuyor.

---

## M12 — Bileşenler ve ekosistem

Erk'in JavaScript'i bir tarayıcı ortamı değil (M6, DOM'un küçük bir alt
kümesi); React, Vue ya da Svelte bileşenleri çalışmaz. Yerine:

- Yerel kontroller motorun içinde (M5'teki form kontrolleri ve
  `<dialog>`, `<details>`, `popover` gibi davranışı olan HTML elemanları)
- HTML ve CSS kalıpları ve Tailwind gibi yalnızca CSS üreten araçlar (CSS
  değişkenleri ve seçicileri Stylo'nun işi)
- Karmaşık bileşenler (tarih seçici, zengin metin düzenleyici) host tarafında,
  Erk'in API'si üzerine sarmalayıcı kütüphaneler olarak

**Kabul:** Belgelenmiş bir bileşen kalıpları kataloğu ve her biri için Chrome
referans sayfası.

---

## Bilerek kapsam dışı

| Ne | Neden |
|---|---|
| Çekirdekte betik; tarayıcı uyumlu bir JS ortamı (Web API'leri, React gibi çatılar) | JS isteğe bağlı bir bağlama (M6) ve DOM'un küçük bir alt kümesi; iş mantığı host'ta |
| Zengin metin düzenleme (`contenteditable`) | M5'in form kontrollerinden çok daha büyük bir iş; bir e-posta istemcisinin yazma penceresi gibi ekranlar Erk'in alanı dışında (p1-embedded §4) |
| Ağ, HTTP, Fetch, çerezler | Host'un işi; motor ağa hiç erişmez |
| Kum havuzu, çoklu süreç, site izolasyonu | İçerik host'un kendisi; mesajlar serileştirilebilir kaldığı için ihtiyaç olursa sonradan eklenebilir |
| Float, clear, tablo düzeni, multi-column, print/paged media | Masaüstü UI'ı flex ile kurulur; ayrıntı css-support.md'de |
| WebExtensions, medya ve DRM, WebRTC, WebXR | Tarayıcı işleri |
| Mobil platformlar | Hedef masaüstü; winit ve vello mobili destekliyor ama dokunma, yazılım klavyesi ve yaşam döngüsü ayrı bir iş, masaüstü oturduktan sonra yeniden değerlendirilir |
| İşletim sistemi entegrasyonu çekirdekte (tepsi, diyalog, bildirim, kısayol, güncelleme) | Host'un işi; `erk new` şablonları hazır host kodu olarak getirir (M8) |
| Kendi font ayrıştırıcı, shaper, görüntü çözücü | skrifa, HarfRust, png, jpeg çözücüleri var |
