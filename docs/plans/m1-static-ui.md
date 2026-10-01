# Erk Engine M1 (Statik UI) Uygulama Planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bir masaüstü ayarlar ekranı maketi (form satırları, flex düzeni,
kenarlıklar, görüntüler, farklı stillerde Türkçe metin) Erk'te doğru çiziliyor
ve Chrome referans testinde skorlu. CSS WPT taban çizgileri yayımlı, fuzz job'ı
yeşil, ikili boyutu bütçenin altında.

**Architecture:** M0'ın hattı değişmez: html5ever → arena DOM → Stylo → Taffy +
Parley → display list → vello_cpu. M1'in özgün işi inline formatting context:
bir blok kutusunun inline içeriği (metin, `<span>`, `<b>`, satır içi görseller)
Parley'de tek bir layout olarak, stil aralıklarıyla ve inline kutularla
şekillenir; Taffy yalnızca bloğun boyunu görür. Anonim kutular DOM'a değil
layout yan tablosuna girer: host'un gördüğü `NodeId`'ler yalnızca kendi
belgesini gösterir (p1-contract §2). Gerekçeler:
[p1-embedded.md](../design/p1-embedded.md), [p1-contract.md](../design/p1-contract.md).

**Tech Stack:** M0'daki sürümler (Stylo 0.20, Taffy 0.14, Parley 0.11.1,
vello_cpu 0.2). Yeni: fontique sistem fontları (Parley'nin `system` özelliği),
png ve jpeg çözücüleri, cargo-deny, cargo-fuzz.

## Global Constraints

- **Sözleşme:** Çekirdek G/Ç, ortam ve saat kullanmaz (`check-core-io.sh`).
  Görüntüler ve CSS `url()` yalnızca host'un kaynak sağlayıcısından gelir.
  Sistem fontlarını okumak da bir G/Ç'dir: font tarama çekirdekte değil
  host tarafında (M1.7'de kararı yazılır).
- **Kapsam:** css-support.md. Float `none` gibi dizilir, tablo algoritması yok.
  Bir özellik matrise "Supported" olarak ancak testini adlandırarak girer.
- **Adım başına PR:** her adım kendi dalında (`m1/...`) ve kendi PR'ında.
  Render'ı değiştiren her adım Chrome skor tablosunu commit gövdesine ve PR
  açıklamasına yazar; yeni render davranışı kendi referans sayfasıyla gelir,
  Chrome görüntüsü ayrı bir commit'te yakalanır.
- **Blitz'ten uyarlama:** blitz-dom 0.3.0-beta.2 (MIT OR Apache-2.0), dosya
  başına kaynak yorumuyla. Blitz'in `resolve_calc_value`'su `unsafe`; Erk'te
  calc değerleri `CalcTable`'dan geçer. Servo MPL-2.0: yalnızca okunur.
- **Test disiplini:** her değişiklik testle başlar; her yeni test, koruduğu
  hatayı üreten bir mutasyonla denenir.
- **Belirleyicilik:** testler gömülü Noto Sans'la çizer; sistem fontları
  testlere hiç girmez.

## Dosya yapısı (M1 sonunda)

```
crates/erk-renderer/src/layout/mod.rs      Taffy ağaçları, anonim kutular (M1.0)
crates/erk-renderer/src/layout/inline.rs   IFC: Parley stil aralıkları, inline kutular (M1.3)
crates/erk-renderer/src/display.rs         kenarlık, yuvarlak köşe, gölge, görüntü öğeleri (M1.6)
crates/erk-renderer/examples/measure.rs    ilk kare ölçümü (M1.0)
crates/erk-renderer/tests/robustness.rs    tohumlu girdi testi ve çökme korpusu (M1.3)
fuzz/                                      cargo-fuzz hedefleri, ayrı nightly (M1.3)
tests/wpt/                                 wptrunner "erk" ürünü ve beklentiler (M1.4)
examples/perf/nodes-1000.html              ölçüm sayfası (M1.0)
.github/size-budget.txt                    ikili boyutu bütçesi (M1.0)
deny.toml                                  lisans izin listesi (M1.6)
```

---

### M1.0: Ölçüm, bütçe ve M0'ın metin düşürmesi

M1'e dokunmadan önce iki şey: nereden başladığımızı ölçmek ve M0'ın metni
sessizce düşürdüğü yeri kapatmak.

- [x] **Step 1: Ölçüm sayfası.** `examples/perf/nodes-1000.html`: bir ayarlar
  listesi, 1000 eleman (bölümler, satırlar, etiketler, değerler), gerçekçi CSS.
- [x] **Step 2: İlk kare ölçümü.** `crates/erk-renderer/examples/measure.rs`:
  bir sayfayı okur, `render_html`'i N kez çalıştırır, ilk (soğuk) çağrının
  süresini ve sonraki çağrıların medyanını yazdırır. Bir örnek olduğu için
  dosya ve saat okuyabilir; çekirdeğin `src`'si değildir. Aşama aşama döküm
  için çekirdeğe saat sokulmaz, profilere bırakılır.
- [x] **Step 3: Ölçümler.** Yayın profiliyle, bu makinede:
  - `erk` ikilisinin boyutu. Varsayılan profille, ayrıca `strip` ve
    `lto = "fat"` + `codegen-units = 1` ile, karar verisi olarak.
  - `erk examples/perf/nodes-1000.html` penceresinin ilk kareden sonra
    boştaki belleği (özel çalışma kümesi).
  - `measure` ile ilk kare ve sıcak kare süreleri.
  Sonuçlar ve makine bilgisi bu planın yürütme notlarına yazılır.
- [x] **Step 4: Boyut bütçesi muhafızı.** CI'da ayrı bir `size` job'ı (ubuntu):
  `cargo build --release -p erk-shell --locked`, ikilinin boyutu
  `.github/size-budget.txt`'teki tavanla karşılaştırılır. Tavan, Linux'ta
  ölçülen boyutun %10 üstüdür (ilk CI çalışmasından alınır). Bütçeyi
  yükseltmek gerekçe ister, düşürmek serbesttir; beklenti dosyasındaki
  `# lowered:` kuralının tersi. Kasıtlı ihlal: tavanı ölçülenin altına çek →
  job kırmızı.
- [x] **Step 5: CSS matrisi muhafızı.** `docs/css-support.md`'deki her
  "Supported" satırın adlandırdığı test depoda tanımlı olmalı
  (`.github/scripts/check-css-support.sh`). Kasıtlı ihlal: bir satıra var
  olmayan bir test adı yaz.
- [x] **Step 6: `erk-renderer` pencere katmanını bilmez.** CI: `cargo tree -p
  erk-renderer --target all --all-features` çıktısında `winit` ve `softbuffer`
  yok (p1-contract §11). Kasıtlı ihlal: `erk-renderer`'a `winit`.
- [x] **Step 7: Anonim blok kutuları (test önce).** Blok ve metin karışık bir
  ebeveynde (`<div>önce<p>blok</p>sonra</div>`) "önce" ve "sonra" bugün
  düşüyor. Ardışık inline içerik (metin ve inline elemanlar) anonim bir
  paragraf kutusuna girer:
  - Anonim kutular layout yan tablosunda, arena kapasitesinin üstündeki
    indekslerde durur; DOM'a hiçbir şey eklenmez.
  - Stilleri ebeveynin hesaplanmış stilinden gelir (M0'daki paragraf
    yaprağıyla aynı basitleştirme; stil aralıkları M1.3'te).
  - Yalnızca boşluktan oluşan diziler kutu üretmez.
  - Display list anonim kutuların metnini ebeveynin kutusunun altında çizer.
  Testler: layout testinde üç kutu alt alta; display list dökümünde "önce" ve
  "sonra" var. Mutasyon: anonim kutu üretmeyi kapat → test kırmızı.
  Yeni referans sayfası `mixed-content.html`; Chrome görüntüsü ayrı commit'te.
- [x] **Step 8:** Matris ve belgeler güncellenir; yürütme notları yazılır.

**Kapsam dışı (M1.3'e):** span stillerinin düzleşmesi. Düzeltmesi IFC'nin
kendisi (Parley stil aralıkları), yarım bir sürümü iki kez yazmak olur.

### M1.1: Tek satır metin — M0'da var

Parley ile şekillenen bir Taffy yaprağı (`a_paragraph_is_one_line_high`).

### M1.2: Satır kırma — M0'da var

Daralan kutuda metin alt satıra iner (`narrow_width_breaks_into_more_lines`,
`a_narrow_container_wraps_the_paragraph`).

### M1.3: Inline formatting context

- [ ] **Step 1:** Blitz 0.3.0-beta.2'nin `layout/construct.rs` (inline ağaç
  kurulumu, `build_inline_layout_into`) ve `layout/inline.rs`
  (`compute_inline_layout`) dosyaları okunur; Erk'in yan tablosuna ve
  `CalcTable`'ına uyarlama planı yürütme notlarına yazılır.
- [x] **Step 2: Stil aralıkları.** Bir bloğun inline içeriği tek bir Parley
  layout'u olur; her inline elemanın stili (renk, kalınlık, italik, boyut,
  font ailesi, `line-height`) kendi metin aralığına uygulanır. Test: `<p>a
  <b>b</b> c</p>`'de "b" kalın yüzle, "a" ve "c" normal.
- [x] **Step 3: Inline kutular.** Satır içi görseller ve `inline-block`
  Parley'nin `InlineBox`'larıyla. Kenarlıklı ve dolgulu `<span>`'lar satırlar
  arasında kırılır.
- [x] **Step 4:** `text-align` (start, end, center, justify), temel
  `vertical-align` (baseline, middle, top, bottom; Blitz'te yok, Erk'in işi).
- [x] **Step 5: Sağlamlık.** `tests/robustness.rs`: sabit tohumlu bir üreteçle
  bozuk HTML ve CSS (kapanmamış etiketler, dev sayılar, derin iç içe
  yapılar, geçersiz UTF-8'den dönüştürülmüş metin) `render_html`'den geçer,
  panik yok. Çökme korpusu `tests/robustness/` altında. `fuzz/` altında
  cargo-fuzz hedefi, sabit bir nightly, CI'da ayrı ve zaman sınırlı job.
- [ ] Referans sayfaları: `inline-styles.html`, `text-align.html`.

### M1.4: Block ve absolute positioning

- [x] `position: relative | absolute | fixed`, `top/right/bottom/left`,
  `z-index` ile boyama sırası (yığın bağlamları, CSS 2 Ek E'nin gerisi).
- [x] Float `none` gibi dizilir: `float: left` bir kutu metni düşürmez (test).
- [x] **WPT altyapısı:** wptrunner'a `erk --screenshot` üzerinden koşan "erk"
  ürünü; `css/CSS2/normal-flow` ve `css/css-position` taban çizgileri JSON,
  gerileme yasağı CI'da.
- [x] **Akış layout'u kararı** (roadmap): normal-flow ve css-text taban
  çizgisindeki kalan testler sınıflanır, karar yazılır.
- [x] Referans sayfası: `positioning.html`.

### M1.5: Flexbox

- [ ] Taffy'nin flexbox'ı doğrulanır: `flex-direction`, `wrap`, `gap`,
  `justify-content`, `align-items`, `flex-grow/shrink/basis`, `order`.
- [ ] `css/css-flexbox` taban çizgisi.
- [ ] Referans sayfası: `flex-form.html` (ayarlar ekranı düzeni).

### M1.6: Renkler, kenarlıklar, görüntüler

- [ ] Display list öğeleri: kenarlık (solid, renkli, kenar başına genişlik),
  yuvarlak köşe (`border-radius`, kırpma dahil), `box-shadow`, `opacity`.
- [ ] Görüntüler: `<img>` ve `background-image` (png, jpeg), sözleşmenin
  kaynak API'siyle (tür + MIME). Demo kabuğun sağlayıcısı: açılan dosyanın
  dizini ve `memory://`; `url("file:///...")` hiçbir şey okumaz (test).
  Yanlış türde yanıt reddedilir (test).
- [ ] **Lisans kapısı:** `deny.toml` ve CI'da `cargo deny check licenses`;
  izin listesi MIT, Apache-2.0, MPL-2.0, OFL-1.1 ve bağımlılık ağacının
  gerektirdiği diğerleri, her biri gerekçesiyle.
- [ ] Referans sayfaları: `borders.html`, `images.html`.

### M1.7: Sistem fontları, HiDPI, text-transform

- [ ] **Karar:** sistem fontlarını kim tarar? Çekirdek G/Ç yapmaz; fontique'in
  sistem taraması host tarafında (`erk-shell`, M3'te `erk`) çalışır ve font
  koleksiyonu çekirdeğe veri olarak verilir. Karar ve gerekçe
  p1-contract'a eklenir.
- [ ] Fallback: CJK ve emoji için sistem fontları; testler yine gömülü fontla.
- [ ] HiDPI: cihaz ölçeği `ErkConfig.scale` / pencereden; layout CSS
  pikselinde, boyama cihaz pikselinde.
- [ ] `text-transform`, elemanın `lang`'ına göre (`icu_casemap`): Türkçede
  `i → İ`, `ı → I` (test).

### M1 kabulü

**Chrome uyumluluk kapısı.** M1'in sorusu "Chrome'la aynı PNG'yi üretebiliyor
muyuz" değil, "Chrome referansına karşı kaç HTML/CSS davranışını belirleyici
olarak aynı üretiyoruz". css-support.md'de "Supported" olan her özellik için
bir referans sayfası; her sayfada Chrome PNG'si ve Chrome geometrisi. CI:

- geometri uyuşmazlığı (1 CSS pikselinden fazla) → kırmızı;
- piksel skoru beklentisinden farklı → kırmızı (iki yönlü mandal; metin kenar
  yumuşatması yüzünden skor %100 değil, beklenti sayfaya özgü);
- bozuk girdide panik → kırmızı (sağlamlık testi ve fuzz, M1.3).

M1'de yeni mimari özellik eklenmez; iş statik UI kapsamı ve bu kapının
genişlemesidir.


- [ ] Ayarlar ekranı maketi (`examples/settings.html`) Chrome referans
  testinde skorlu.
- [ ] `css/CSS2/normal-flow`, `css/css-flexbox`, `css/css-position`,
  `css/css-text` taban çizgileri yayımlı, gerileme yasağı CI'da.
- [ ] Fuzz job'ı yeşil; ikili boyutu bütçenin altında; bellek ve ilk kare
  ölçümü M1 sonunda tekrarlanıp M1.0'la karşılaştırılmış.
- [ ] Akış layout'u kararı gerekçesiyle belgelenmiş.
- [ ] `roadmap.md`'de M1 "Bitti".

---

## Yürütme Notları

*(Adımlar yürütüldükçe, planın yanlış çıkan varsayımları ve doğrulanan
gerçeklerle doldurulur.)*

### M1.0 yürütme notları (2026-09-30)

**Ölçüm makinesi:** Intel Core i7-10750H (6 çekirdek, 2,60 GHz), 15,8 GB RAM,
Windows 11 Pro, %125 ekran ölçeği. Rust 1.98.1, varsayılan yayın profili
(`cargo build --release`, derleme 328 sn).

| Ölçüm | Değer |
|---|---|
| `erk.exe` boyutu, varsayılan yayın profili | 14.899.200 bayt (14,9 MB) |
| ... `strip = "symbols"` ile | 14.924.288 bayt: Windows'ta etkisiz, MSVC hata ayıklama bilgisini zaten ayrı PDB'ye koyuyor |
| ... `lto = "fat"` + `codegen-units = 1` ile | 11.431.936 bayt (%23 küçük) |
| ... ayrıca `opt-level = "s"` ile | 9.498.624 bayt (%36 küçük) |
| `nodes-1000.html` (1006 eleman), `render_html` ilk çağrı | 116,85 ms |
| ... sonraki 30 çağrının medyanı (min / maks) | 70,95 ms (62,79 / 83,29) |
| `merhaba.html`, ilk çağrı / medyan | 8,32 ms / 6,88 ms |
| `erk nodes-1000.html` penceresi, ilk kareden 3 sn sonra | özel bellek 13,0 MB, çalışma kümesi 30,1 MB, tepe 40,3 MB |
| `erk merhaba.html` penceresi, aynı ölçüm | özel bellek 12,6 MB, çalışma kümesi 29,6 MB, tepe 35,5 MB |

Okuma:

- **"< 5 MB" hedefi gerçekçi değildi.** Stylo, html5ever, Parley ve vello'lu
  bir ikili varsayılan profille 14,9 MB; en küçük denenmiş profille bile
  9,5 MB. Bütçe bu yüzden bir hedef değil, ölçümden konur.
- **Profil kararı M8'e (ürünleşme) kaldı.** `opt-level = "s"`'in boyutu
  %36 küçülttüğü görüldü, ama çizim hızına etkisi ölçülmedi, LTO ise yayın
  derlemesini birkaç kat uzatıyor. Bütçe varsayılan profil üzerinden işler;
  profil değişirse bütçe gerekçesiyle yeniden konur.
- **1000 elemanlı bir sayfanın tam yeniden çizimi ~71 ms**, yani 60 Hz'in
  dört katı. Bu, M2'nin tam yeniden hesaplamayla neden "bilerek kaba"
  olduğunu ve M5'teki artımlı işin neden şart olduğunu sayıyla gösteriyor.
  İlk çağrıdaki fark (~46 ms), gömülü fontların ve Stylo'nun bir kerelik
  kurulumu.
- Bellek iki sayfa arasında neredeyse aynı: tabanı pencere, softbuffer ve
  gömülü fontlar belirliyor, belge değil.
- İlk bellek okuması 0 döndü: süreç bilgisi pencere açılmadan okunmuştu.
  Ölçüm, pencere başlığı görünene kadar ve sürecin hâlâ açık olduğu
  denetlenerek tekrarlandı.

**Boyut bütçesi:** Linux ikilisinin boyutu bu makinede ölçülemiyordu;
dosyaya "geçici" işaretli 20 MB'lık bir tavan kondu. PR #7'deki ilk `size`
çalışması Linux ikilisini **22.023.288 bayt** ölçtü ve tavanı aştı: tahmin
yanlıştı. Linux ikilisi Windows'takinden (14,9 MB) büyük; ELF'te kalan
sembol tabloları ve winit'in X11/Wayland kodu olası nedenler, ayrıştırılmadı.
Bütçe ölçümün %10 üstüne, **24.225.617 bayta** kondu.

Bu yükseltme, muhafızın kendi hatasını da buldu. Betik gerekçeyi dosyadaki
ilk `# raised:` eşleşmesinden okuyordu; bu, açıklama başlığındaki örnek
satırdı. Yeni ve taban dosyada aynı örnek okununca her yükseltme "eski
gerekçe" sayılıp reddediliyordu, yani muhafız hiçbir yükseltmeye izin
vermiyordu. Gerekçe artık yalnızca bütçe satırından okunuyor. Denenen
durumlar: gerekçeli yükseltme (0), gerekçesiz (1), boş gerekçe (1), ikilinin
altına düşürme (1), gerekçesiz düşürme (0), taban gerekçesini yeniden
kullanan yükseltme (1).

**Anonim kutular:** karışık bir ebeveyndeki her inline dizi, arena
kapasitesinin üstündeki bir indekste anonim bir paragraf kutusu olur; DOM'a
hiçbir şey eklenmez. Display list onları ebeveynin kutusunun altında çizer,
ebeveynin görünürlüğünü devralırlar. Mutasyon: anonim kutu üretimi
kapatılınca `text_beside_blocks_gets_anonymous_boxes` kırmızı. Yeni referans
sayfası `mixed-content` %97,95; kutular Chrome'la aynı yerde, tek gerçek
sapma düz yazıyla çizilen `<b>kalın</b>` (M1.3).

**Chrome 154:** yakalama makinesinde Chrome 153'ten 154'e yükselmişti. Yeni
sayfa eski sürümün görüntüleriyle karışmasın diye bütün sayfalar yeniden
yakalandı. 154, beş sayfanın beşini de bayt bayt aynı çizdi; yalnızca
`VERSION.txt` değişti.

**Kasıtlı ihlaller:**

- CSS matrisi: var olmayan bir test adı, test adı olmayan bir satır, var
  olmayan bir test dosyası → üçü de 1 ile çıktı; geri alınınca 0.
- `erk-renderer`'a `winit` bağımlılığı → adım 1 ile çıktı; geri alınınca 0.
- Boyut bütçesi: tavanın üstündeki, altındaki, bütçesiz dosya ve eksik
  ikili sahte bir dosyayla denendi (1, 0, 1, 1); yükseltme ve düşürme
  kuralları yukarıda.

### M1.3 yürütme notları (2026-09-30, sürüyor)

**Step 2 tamam, stil aralıkları.** Paragraf artık tek bir metin ve ondan
farklı stildeki aralıklar. Her metin düğümü içinde durduğu elemanın stilini
alır; boyut, kalınlık, renk ya da satır yüksekliği bloktan farklı olan her
aralık Parley'ye `push(özellik, aralık)` ile verilir. Boşluk parçalar
boyunca çöker ve çöken boşluk önceki parçaya aittir (`a <b>b</b>`'deki boşluk
kalın değil).

| Plan ne diyordu | Gerçek |
|---|---|
| Stil aralıkları Blitz'in `construct.rs`'inden uyarlanacak | Önce Erk'in kendi yapısıyla yazıldı: aralıklar metin düğümlerinden doğrudan çıkıyor, Blitz'in inline ağacı henüz gerekmedi. Blitz'e inline kutular (Step 3) için dönülecek; Step 1 o işin başında |
| — | Parley glyph run'ları stile göre bölüyor, ama renk değişimi şekillendirme run'ını bölmüyor ve dışarı açılan metin aralığı bütün run'ınki. Her glyph run'ın kendi aralığı, run'ın kümeleri sırayla tüketilerek çıkarılıyor (`glyph_run_ranges`). Display list dökümü de her glyph run için bütün run'ın metnini yazıyordu; o da düzeldi |
| — | İtalik yok: gömülü fontlarda italik yüz yok, sentetik eğim testlerde belirleyiciliği bozmasın diye bilerek uygulanmadı; sistem fontlarıyla (M1.7) gelir |

Mutasyonlar: aralıklar hiç uygulanmayınca iki test de kırmızı; çöken boşluk
sonraki parçaya verilince kalınlık testi kırmızı.

Referans sayfaları: `mixed-content` %97,95'ten %98,33'e çıktı (kalın kelime
artık kalın). Yeni `inline-styles` %48,60: renkler, kalınlıklar, boyutlar ve
satır sonları Chrome'la örtüşüyor; skor, metinle dolu sayfalarda glif
kenarlarındaki kenar yumuşatma farkı yüzünden `paragraphs` gibi düşük.

1000 elemanlı sayfanın süresi değişmedi (sıcak medyan 68,5 ms).

**Step 4, `text-align` (2026-10-01).** Bloğun `text-align`'ı Parley'nin
hizalamasına çevriliyor: start, end, left, right, center, justify ve `align`
özniteliğinin ürettiği eski `-moz-` değerleri. Parley iki yana yaslamayı
satırın ölçülerini değil kümelerin genişliğini büyüterek yapıyor; testler bu
yüzden glif run'larının gerçek ucunu, satır sonundaki (kenardan taşan)
boşluğu düşerek ölçüyor. Yeni `text-align` referans sayfası %93,48, kutuları
6/6 Chrome'la aynı. **`vertical-align` yapılmadı:** satır içi kutulara bağlı,
Step 3 ile gelir.

**Step 5, sağlamlık (2026-10-01).** Deneme, gerçek bir çökme buldu: 1000 düzey
iç içe `<div>`, renderer iş parçacığının yığınını taşırıp süreci
öldürüyordu (layout her düzeyde bir kez özyineleniyor). Chrome'da ölçüldü:
ayrıştırıcı derinliği `<body>`'nin 511 altında kesiyor, daha derin elemanlar
kardeş oluyor (Blink, `kMaximumHTMLParserDOMTreeDepth`). Erk'in ayrıştırıcısı
artık aynı kuralı uyguluyor, ağaç Chrome'unkiyle aynı. 512 düzey hata
ayıklama derlemesinde yine de 2 MiB'tan fazla yığın istiyor; renderer iş
parçacığına 16 MiB verildi (yalnızca adres alanı).

| Plan ne diyordu | Gerçek |
|---|---|
| Sağlamlık testi bozuk HTML/CSS'te panik aramak | Panikten önce bir çökme çıktı: derin iç içelik yığını taşırıyordu. Yığın taşması panik değil süreç ölümüdür, `catch_unwind` yakalayamaz; derin belge testi bu yüzden gerçek renderer iş parçacığından geçiyor |
| cargo-fuzz hedefi bu adımda | **Yapılmadı:** ayrı bir nightly ve CI job'ı istiyor; sabit tohumlu üreteç ve korpus bugünkü kapıyı karşılıyor. Fuzz hedefi M1 bitmeden ayrı bir PR'da |
| — | Sözleşmeye açık bir soru eklendi: M3'te layout UI iş parçacığında çalışacak, ama Windows'ta ana iş parçacığının yığını 1 MB. Ya layout'un özyinelemesi kaldırılacak ya da gereken yığın ölçülüp belgelenecek |

Mutasyonlar: derinlik sınırı ya da büyük yığın kaldırılınca test süreci yığın
taşmasıyla çöküyor; bir kontrol karakterinde enjekte edilen panik, belgesiyle
birlikte raporlanıyor.

**Step 3, satır içi kutular (2026-10-01).** Satır içi bir elemanın yatay
kenar boşluğu, kenarlığı ve dolgusu, iki ucunda o genişlikte birer Parley
satır içi kutusu oluyor; arka planı her satırda, elemanın metni ve kutuları
üzerinden ayrı bir dikdörtgen olarak boyanıyor. `inline-block`,
`inline-flex` ve `inline-grid` paragrafın Taffy çocuğu: önce kendi
boyutunda yerleşiyor, sonra o boyutta bir satır içi kutu olarak satıra
giriyor. Yerleşim Blitz'in `layout/inline.rs`'indeki yaklaşımı izliyor ama
Erk'in ağacına ve `CalcTable`'ına göre yeniden yazıldı. Yeni `inline-boxes`
referans sayfası %75,10; kutuların 8/8'i Chrome'la 1 px içinde, arka planlar
piksel piksel aynı, kalan fark gliflerin kenar yumuşatması.

| Plan ne diyordu | Gerçek |
|---|---|
| Satır içi kutular Parley'nin `InlineBox`'larıyla | Kutular Parley'ye **yüksekliksiz** veriliyor. Parley bir satıra tek satır yüksekliği verip boşluğu en uzun içeriğin çevresine bölüyor; CSS her satır içi kutuya taban çizgisinin çevresinde kendi yerini veriyor (CSS 2 §10.8). Parley'nin yolu 30 px'lik bir kutunun üstünü önceki satırın içine taşırıyordu. Atom içeren satırların kutusu Erk'te hesaplanıyor: dayanak (strut), metnin kendi yüksekliği ve her atomun taban çizgisinin üstü ve altı; sonraki satırlar aşağı kayıyor |
| Satır içi görseller bu adımda | **Yapılmadı:** Erk henüz hiçbir görsel yüklemiyor. Satır içi görsel `<img>` ve kaynak callback'iyle M1.6'da gelir; atom yolu hazır |
| `vertical-align` bu adımda | **Yapılmadı:** satır kutusu artık Erk'te hesaplandığı için atomlarda doğrudan eklenebilir; metin aralıklarında glif run'larının dikey kaydırılmasını da istiyor. Ayrı PR |
| — | Arka plan kenarları metne göre kesirli x'e düşüyordu ve her kenar bir sütun karışık piksel oluyordu; Chrome piksele hizalıyor. Hizalanınca skor %74,65'ten %75,10'a çıktı ve fark görüntüsünde arka plan kenarı kalmadı |
| — | Boyama artık son layout'un kırdığı satırları kullanıyor; eskiden her paragraf boyama için yuvarlanmış genişlikte bir kez daha şekillendiriliyordu. Bütün referans skorları aynı kaldı; 1000 elemanlı sayfanın sıcak medyanı bu makinede aynı oturumda 64 ms'den 53 ms'ye indi |
| — | Bilinen sınırlar: yüzdelik dolgu ve kenar boşluğu satır içi elemanlarda sıfır sayılıyor (satır kırma sırasında kapsayıcı genişliği yok); dolgulu bir elemanın uçları kelime içinde de satır kırma fırsatı (Parley her kutudan sonra kırabiliyor); blok kapsayıcı bir `inline-block` CSS'in istediği son satırın değil ilk satırın taban çizgisini kullanıyor (Taffy blokları yalnızca ilk taban çizgisini iletiyor) |

Mutasyonlar (11/11 yakalandı): uç kutularının genişliği sıfır; arka plan
dolguyu yok sayıyor; çöken boşluk hep elemanın dışında; satır kutusu
düzeltmesi kapalı; atomun taban çizgisi yok sayılıyor; anonim kutunun
konumu eklenmiyor; kutusuz elemanların içinden geçilmiyor; satır içi arka
planlar metinden sonra; satır sonundaki boşluk arka plan alıyor; satır
kayması boyanmıyor; arka planlar piksele hizalanmıyor. Sağlamlık üretecine
`inline-block`, `inline-flex` ve dev dolgulu, negatif kenar boşluklu satır
içi arka planlar eklendi.

**`vertical-align` (2026-10-01).** Stylo 0.20'de `vertical-align` bir
kısaltma: CSS Inline 3'ün `alignment-baseline` (baseline, middle, text-top,
text-bottom) ve `baseline-shift` (sub, super, top, center, bottom, uzunluk,
yüzde) uzun hâllerine açılıyor; Erk ikisini birlikte okuyor. Satır içi bir
elemanın yükselmesi Parley fırçasına (`TextBrush`) giriyor: farklı
yükseklikteki metin kendi glif run'larını alıyor, boyama onları kaydırıyor,
satır kutusu yükseltilmiş metne yer açıyor. Atomlar aynı hesaptan geçiyor;
`top`, `center` ve `bottom` satır kutusu kurulduktan sonra yerleşiyor ve
daha uzunlarsa satırı kendi kenarlarından uzak yöne büyütüyor. Yeni
`vertical-align` referans sayfası %68,11; kutuların 10/10'u Chrome'la 1 px
içinde, arka planlar ve kutular piksel piksel aynı, kalan fark gliflerin
kenar yumuşatması.

| Plan ne diyordu | Gerçek |
|---|---|
| Temel `vertical-align` | Atomlarda değerlerin hepsi, satır içi elemanlarda satır kutusuna göre olanlar (`top`, `center`, `bottom`) dışında hepsi. Onlar satır içi elemanda `baseline` gibi diziliyor; satır kutusu kurulduktan sonra metin run'larını taşımak ayrı bir iş |
| — | `sub` ve `super` Blink'in ofsetleri: ebeveyn yazı tipinin beşte biri artı bir piksel aşağı, üçte biri artı bir piksel yukarı. Chrome bunları 1/64 px'lik layout biriminde hesaplıyor (6,333 → 405/64). Kesirsiz hâliyle `<sup>`'lu satır Chrome'unkinden 0,017 px uzun çıkıyordu ve sayfanın aşağısındaki her kutu ters yöne yuvarlanıyordu. Ofsetler artık o birime kırpılıyor |
| — | Taffy her kutunun konumunu ebeveynine göre ayrı yuvarlıyor; ebeveynin piksel kesri kayboluyor. Chrome mutlak konumu piksele oturtuyor. Erk artık her kutunun yuvarlanmış mutlak konumundan ebeveyninkini çıkarıyor; önceki sayfaların bütün konumları tam sayı olduğu için skorları değişmedi, `vertical-align` sayfası %67,81'den %68,11'e çıktı ve fark görüntüsünde kutu kenarı kalmadı |
| — | Satır içi görseller hâlâ M1.6'da (`<img>` ile); M1.3'ün kalan tek işi oydu, bu yüzden M1.3 bitti sayıldı. cargo-fuzz job'ı M1 bitmeden ayrı bir PR'da |

Mutasyonlar (10/10 yakalandı): yükselme boyanmıyor; yükseltilmiş metin
satırı büyütmüyor (ilk denemede kaçtı: test yalnızca "bir satırdan uzun"
diyordu ve `sub` satırı zaten büyütüyordu; yerine kesin yükseklik testi
geldi); yükseltilmiş arka plan taban çizgisinde kalıyor; `middle` x
yüksekliğini yok sayıyor; `text-top` alçalmayı kullanıyor; `bottom` üste
yerleşiyor; uzun bir satır hizalı kutu satırı büyütmüyor; layout birimi
yok; `super` beşte bir kullanıyor; konumlar ebeveyne göre yuvarlanıyor.
Sağlamlık üretecine uç `vertical-align` değerleri eklendi.

**M1.4, konumlandırma (2026-10-01).** Plan adımı ikiye bölündü: bu PR
konumlandırma, `z-index` ve float; WPT altyapısı ve akış layout'u kararı
ayrı bir PR'da (karar WPT taban çizgisine bağlı). Yeni `positioning`
referans sayfası %97,39; kutuların 13/13'ü Chrome'la 1 px içinde, sabit
alt çubuk ve `z-index` sırası piksel piksel aynı.

| Plan ne diyordu | Gerçek |
|---|---|
| Absolute positioning'i Taffy'yle doğrulamak | Taffy absolute bir kutuyu her zaman **ebeveynine** göre yerleştiriyor; CSS'te kapsayıcı blok en yakın konumlandırılmış ata ya da görüntü alanı. Erk absolute ve fixed elemanları akıştan çıkarıp kapsayıcı bloklarının Taffy çocuğu yapıyor, layout'tan sonra her kutunun konumunu en yakın kutulu DOM atasına göre yeniden hesaplıyor (boyama ve host DOM'u yürür). Bu genelleme atomların anonim kutu düzeltmesinin de yerini aldı. Kök kutu artık görüntü alanı boyutunda: ilk kapsayıcı blok o |
| — | Absolute bir eleman paragrafı bölüyordu: Stylo onu bloğa çeviriyor, Erk de blok çocuk sanıp metni anonim kutulara ayırıyordu. Artık satır içi içerikten tamamen çıkıyor |
| — | Konumlandırılmış bir metin bloğu paragraf yaprağı olsaydı absolute çocuklarını Taffy yerleştiremezdi; böyle bir blok blok kutusu olarak kalıyor, metni anonim bir paragrafta |
| — | stylo_taffy `static` ve `sticky`'yi Taffy'nin relative konumuna eşliyor, yani `top`/`left` static bir kutuyu da kaydırıyordu; artık yok sayılıyor |
| Float `none` gibi dizilir | Taffy'nin float özelliği açıktı ve float'u gerçekten yüzdürüyordu: sonraki paragraf float'un yanında, y=0'da başlıyordu ve Parley satırları float'un çevresinden dolaşmadığı için metin float'un üstüne çizilirdi. Taffy'ye artık `float: none` veriliyor |
| `z-index` ile boyama sırası | Yığın bağlamları: negatif `z-index`'liler, akıştaki bloklar, satır içi içerik, `auto`/0, pozitifler. Her konumlandırılmış eleman bir bütün olarak boyanıyor; `z-index: auto`'nun içindeki konumlandırılmış torunlar CSS'te dış bağlama katılabilir, Erk'te içeride kalıyor |
| — | Bilinen sınırlar: bütün inset'ler `auto` olan absolute bir kutu, akışta duracağı yerde değil kapsayıcı bloğun içerik kenarında; satır içi bir eleman kapsayıcı blok olmuyor; `sticky` kaydırma gelene kadar `static` |

Mutasyonlar (10/10 yakalandı): fixed konumlandırılmış ataya gidiyor;
absolute elemanlar akışta kalıyor; konumlar layout ebeveynine göre kalıyor;
static inset'lerini koruyor; float yüzüyor; ilk kapsayıcı blok içerik
boyunda; konumlandırılmış metin bloğu paragraf oluyor; konumlandırılmışlar
akışın içinde boyanıyor; `z-index` yok sayılıyor; negatif `z-index` akışın
üstünde. Sağlamlık üretecine uç konumlandırma değerleri eklendi.

**M1.4, WPT (2026-10-01).** WPT `5cd8e3f` (2026-09-30), seyrek klon:
`tests/wpt/dirs.txt`'teki iki test dizini ve referans/destek dizinleri. İlk
taban çizgisi:

| Dizin | Reftest | Geçen | Oran | Boş karede geçen |
|---|---|---|---|---|
| `css/CSS2/normal-flow` | 746 | 319 | %42,8 | 6 |
| `css/css-position` | 251 | 41 | %16,3 | 6 |

Çökme yok; iki koşu birebir aynı; 997 test bu makinede yaklaşık 20 sn.
"Boş karede geçen", testin karesi tek renkken geçenler: iki taraf da hiçbir
şey çizmediği için eşit çıkmış olabilirler, sayı bu yüzden ayrıca
yazılıyor.

| Plan ne diyordu | Gerçek |
|---|---|
| wptrunner'a `erk --screenshot` üzerinden koşan "erk" ürünü | Erk'in kendi koşturucusu, `crates/erk-wpt` (kullanıcı kararı). Erk betik çalıştırmadığı için WPT'den yalnızca reftest'ler sayılıyor; wptrunner bunun için Python, bir tarayıcı sürücü katmanı ve test başına bir süreç isterdi. Koşturucu `render_html`'i süreç içinde çağırıyor, WPT'nin çoklu referans ve fuzzy kurallarını (docs/writing-tests/reftests.md) uyguluyor |
| Taban çizgileri JSON | Düz metin, `test DURUM` satırları: düşüşün gerekçesi Chrome beklentilerindeki gibi satırına yazılıyor, JSON yorum taşımıyor. Muhafızı `check-wpt-expectations.sh` |
| — | `.xht` testleri XML olarak yazılmış, Erk'in XML ayrıştırıcısı yok. Koşturucu XML ayrıştırmanın iki farkını önceden uyguluyor: CDATA işaretlerini atıyor (yoksa `<style>`'ın ilk kuralı kayboluyordu) ve boş olmayan kendiliğinden kapanan etiketleri (`<div/>`) kapatıyor |
| — | Testlerin bir kısmı başka dizinlerdeki referanslara bakıyor (`css/CSS2/tables/reference`, `css/CSS2/positioning`); eksik bir referans "düştü" değil yapılandırma hatası sayılıyor ve koşu duruyor. `dirs.txt`'te bu dizinler `support:` önekiyle: çekiliyor ama koşulmuyor |
| — | Kaynak yüklenmiyor: dış stil sayfası, görüntü ya da web fontu isteyen test düşüyor ve taban çizgisi bunu kaydediyor. Kaynak API'si M1.6'da |
| `css/css-text` taban çizgisi | **Yapılmadı:** indirme izni iki dizin içindi ve `css-text` çok büyük; M1 kabulünün parçası olarak ayrı PR'da |

**Akış layout'u kararı:** roadmap'e yazıldı. Düşen 427 normal-flow testinin
her biri desteklenmeyen ya da planlanmayan bir özelliği kullanıyor;
hiçbirine dokunmadan düşen test yok. Karar geçici olarak Taffy'de kalmak;
sınıflama M1.6'dan sonra tekrarlanır.

Mutasyonlar: koşturucuda fuzzy'yi yok saymak, iyileşmeyi raporlamamak,
CDATA'yı bırakmak, `mismatch`'i tanımamak (4/4); muhafızda gerekçesiz
düşüş, boş gerekçe, WPT commit'i aynıyken PASS satırını silmek yakalandı,
gerekçeli düşüş geçti.
