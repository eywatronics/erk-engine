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

- [x] Taffy'nin flexbox'ı doğrulanır: `flex-direction`, `wrap`, `gap`,
  `justify-content`, `align-items`, `flex-grow/shrink/basis`, `order`.
- [x] `css/css-flexbox` taban çizgisi.
- [x] Referans sayfası: `flex-form.html` (ayarlar ekranı düzeni).

### M1.6: Renkler, kenarlıklar, görüntüler

- [x] Display list öğeleri: kenarlık (solid, renkli, kenar başına genişlik),
  yuvarlak köşe (`border-radius`, kırpma dahil), `box-shadow`, `opacity`.
- [x] Görüntüler: `<img>` ve `background-image` (png, jpeg), sözleşmenin
  kaynak API'siyle (tür + MIME). Demo kabuğun sağlayıcısı: açılan dosyanın
  dizini ve `memory://`; `url("file:///...")` hiçbir şey okumaz (test).
  Yanlış türde yanıt reddedilir (test).
- [x] **Lisans kapısı:** `deny.toml` ve CI'da `cargo deny check licenses`;
  izin listesi MIT, Apache-2.0, MPL-2.0, OFL-1.1 ve bağımlılık ağacının
  gerektirdiği diğerleri, her biri gerekçesiyle.
- [x] Referans sayfaları: `borders.html`, `images.html`.

### M1.7: Sistem fontları, HiDPI, text-transform

- [x] **Karar:** sistem fontlarını kim tarar? Çekirdek G/Ç yapmaz; fontique'in
  sistem taraması host tarafında (`erk-shell`, M3'te `erk`) çalışır ve font
  koleksiyonu çekirdeğe veri olarak verilir. Karar ve gerekçe
  p1-contract'a eklenir.
- [x] Fallback: CJK ve emoji için sistem fontları; testler yine gömülü fontla.
- [x] HiDPI: cihaz ölçeği `ErkConfig.scale` / pencereden; layout CSS
  pikselinde, boyama cihaz pikselinde.
- [x] `text-transform`, elemanın `lang`'ına göre (`icu_casemap`): Türkçede
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


- [x] Ayarlar ekranı maketi (`examples/settings.html`) Chrome referans
  testinde skorlu.
- [x] `css/CSS2/normal-flow`, `css/css-flexbox`, `css/css-position`,
  `css/css-text` taban çizgileri yayımlı, gerileme yasağı CI'da.
- [x] İkili boyutu bütçenin altında; bellek ve ilk kare ölçümü M1 sonunda
  tekrarlanıp M1.0'la karşılaştırılmış.
- [x] Fuzz job'ı yapılandırılmış ve CI'da yeşil.
- [x] **Fuzz job'ının bir çökmeyi yakaladığı doğrulanmış:** kasıtlı bir
  paniğin job'ı kırdığı ve girdinin artifact olarak saklandığı ayrı bir CI
  koşusunda görülmüş. Yapılandırılmış olması bunu göstermez; muhafız
  ilkesinin kasıtlı ihlal kuralı. (2026-10-02, taslak PR #24, koşu
  37002635739.)
- [x] Akış layout'u kararı gerekçesiyle belgelenmiş.
- [x] `roadmap.md`'de M1 "Bitti".

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

**M1.5, flexbox (2026-10-01).** `css/css-flexbox` WPT'ye eklendi (8,7 MB,
1012 reftest). Hiçbir şey değiştirilmeden 480'i geçiyordu (%47,4); adım
sonunda 538 (%53,2). Yeni `flex-form` referans sayfası (bir ayarlar ekranı:
kenar menü, sağa yaslı durum etiketi, etiketli satırlar, sarılan etiket
listesi, `order`'la yer değiştiren düğmeler) %97,21; kutuların 33/33'ü
Chrome'la 1 px içinde.

| Plan ne diyordu | Gerçek |
|---|---|
| Taffy'nin flexbox'ı doğrulanır | Büyüme, küçülme, temel, `gap`, sarma ve hizalama doğru çıktı (testli). Bulunanlar: (1) Taffy'de `order` yok, öğeleri çocuk sırasıyla diziyor; Erk flex ve grid kapsayıcıların çocuklarını `order`'a göre kararlı sıralıyor. Boyama hâlâ belge sırasında, CSS sıralanmış belge sırası istiyor (yalnızca örtüşmede fark eder). (2) Yalnızca metin içeren bir flex kapsayıcı paragraf yaprağı oluyor, `justify-content` metne hiç uygulanmıyordu; artık yalnızca blok kapsayıcılar paragraf oluyor, flex ve grid'de metin anonim bir öğe |
| — | **Statik konum (M1.4'ün bilinen sınırı).** Düşen flexbox testlerinin 96'sı aynı kalıbı kullanıyordu: inset'leri `auto` olan absolute bir kırmızı kutu, testin geçmesi gereken yeşilin arkasında. Erk onu kapsayıcının içerik kenarına koyduğu için kırmızı görünüyordu. Artık blok düzeyinde sıfır boyutlu bir yer tutucu, satır içinde sıfır genişlikli bir çapa, absolute elemanın akışta duracağı yeri işaretliyor; layout'tan sonra kutu `auto` olan eksende oraya taşınıyor. Flex ya da grid ebeveyn kapsayıcı bloğun kendisiyse statik konumu Taffy hesaplıyor (tek flex öğesi gibi), değilse kutu ebeveynin içerik kenarında. Bu adım üç dizinde 68 testi geçirdi |
| — | Statik konum bir testi düşürdü: `position-absolute-dynamic-static-position-inline` elemanı betikle `display: block`'a çeviriyor; Erk betik çalıştırmıyor, eleman satır içinde kalıyor. Eskiden statik konum yok sayıldığı için rastlantıyla geçiyordu; taban çizgisinde gerekçesiyle |
| — | Kalan düşüşler: 529 düşen flexbox testinden 484'ü desteklenmeyen ya da planlanmayan bir özellik kullanıyor (float 127, görüntü 97, betik 49, dış stil sayfası 45, kenarlık 35, tablo 35, `writing-mode` 30, gradyan 15, `flex-wrap: balance` taslağı 16 ...). Kalan 45'in kümeleri: `gap` ile sarma ve yüzdeler, taban çizgisi hizalaması, `margin: auto`, yüzdelik yükseklikler, `order`'la boyama sırası, içsel boyutlar. Bunlar M1 kabulüne kadar açık |

Mutasyonlar (7/7 yakalandı): flex kapsayıcıda metin paragraf oluyor;
`order` yok sayılıyor; statik konumlar yok sayılıyor; blok düzeyi absolute
satırının altında değil üstünde başlıyor; flex kapsayıcı blok olsa da içerik
kenarı; iki eksen birden statik konumu alıyor; satır içi çapa satır başına
konuyor. Sağlamlık üretecine uç flex değerleri eklendi.

**İnceleme raporu: `docs/reviews/wpt_commit_review.md` (2026-10-01, #17).**
İlk rapor; inceleme raporları kuralı (proje kuralları) bununla başladı.

| Rapor ne diyordu | Karar |
|---|---|
| `elements` etiketi ilk `>`'de bitiyor; tırnaklı bir değerdeki `>` (`title="a > b"`) etiketi erken kapatır | **Geçerli, düzeltildi.** Etiketin sonu tırnak dışındaki ilk `>` (`tag_end`); kendiliğinden kapanan etiketleri açan XHTML döngüsü de aynı yardımcıyı kullanıyor. Yan bulgu: o döngü her `<`'yi etiket sayıyordu; yorumdaki bir kesme işareti (`don't`) tırnak sanılınca on XHTML testi düştü. Yorumlar artık bütün olarak geçiyor, tırnak denetimi yalnızca gerçek etiketlerde |
| CDATA işaretleri dosyanın her yerinden siliniyor; stil dışındaki CDATA'yı bozabilir | **Geçerli, düzeltildi.** `<style>`/`<script>` içinde yalnızca işaretler siliniyor, başka yerde bölüm metin olarak kaçışlanıyor (`<![CDATA[<b>]]>` bir eleman olmuyor). CSS dizgesinin içindeki bir CDATA işareti ayırt edilmiyor; belgelendi |
| İş parçacığı modeli, panik yakalama, fuzzy hesabı | Artı olarak not edildi; değişiklik yok |

Taban çizgisi değişmedi: 2009 sonucun hepsi aynı.

**İnceleme raporu: `docs/reviews/flexbox_commit_review.md` (2026-10-01, #18).**

| Rapor ne diyordu | Karar |
|---|---|
| `resolve_cdata` `"<style"` arıyor; `< style` gibi boşluklu bir yazım gözden kaçabilir | **Geçersiz:** HTML'de de XML'de de `<`'den sonra boşluk gelirse etiket değil metindir; `< style` aramamak doğru. Raporun dokunduğu yerde gerçek bir uç durum vardı ve düzeltildi: `"<style"` araması `<styles>`'ın başını da yakalıyordu; artık adın ardından boşluk, `>` ya da `/` gelmeli (`find_token`) |
| `tag_end`'de öznitelik değerlerinin bitişiğindeki beklenmedik semboller kenar durum doğurabilir | **Kısmen geçerli:** rapor örnek vermiyor; bulunan somut durum tırnaksız bir değerin içindeki kesme işaretiydi (`title=it's`), tırnak açılışı sanılıyordu. Artık tırnak yalnızca `=`'den hemen sonra (boşluklar arada olabilir) bir değer açıyor |

İki düzeltme de testli ve mutasyonla denendi (2/2); WPT sonuçları değişmedi.

**M1.6, kenarlıklar ve efektler (2026-10-01).** Adım ikiye bölündü: bu PR
kenarlık, yuvarlak köşe, gölge ve `opacity`; görüntüler, kaynak API'si ve
lisans kapısı ayrı PR'da (yeni çözücü bağımlılıkları lisans kapısıyla
birlikte girmeli). Yeni `borders` referans sayfası %92,30; kutuların
17/17'si Chrome'la 1 px içinde.

| Plan ne diyordu | Gerçek |
|---|---|
| Kenarlık (solid, renkli, kenar başına genişlik) | Kenarlık kenarlık kutusuyla dolgu kutusu arasındaki halka. Tek renkte halka even-odd ile doluyor; farklı renklerde halka kırpılıp her kenar dış köşeden iç köşeye uzanan kendi yamuğuyla boyanıyor (Chrome'un çapraz birleşimi). `none`/`hidden` dışındaki her stil düz çiziliyor. Satır içi elemanlarda da: üst ve alt her satırda, sol yalnızca ilk, sağ yalnızca son parçada. Tuvale arka planını veren eleman (`body`) da kenarlığını çiziyor |
| `border-radius`, kırpma dahil | Eliptik köşeler, yüzdeler; komşu yarıçaplar kenarı aşarsa hepsi birlikte küçültülüyor (CSS Backgrounds 3 §5.5). Arka plan köşeye göre kırpılıyor; çocukların kırpılması `overflow` ister, M2'de |
| `box-shadow` | Dış gölgeler (konum, bulanıklık, yayılma, birden çok gölge), kutunun içine hiç düşmeden (kutu kırpma katmanıyla dışarıda bırakılıyor). İlk gölge en üstte. **Bulanıklık:** CSS standart sapmayı yarıçapın yarısı sayıyor; vello_cpu'nun parametresi ölçümde σ·√2 çıktı (3 → σ ≈ 2,1; 5,6 → σ ≈ 4,0). Doğrudan σ verilince gölgenin kenarı Chrome'unkinden üçte bir dar düşüyordu; şimdi kenar profili Chrome'la piksel piksel çakışıyor (57/58, 76/76, 100/101, 128/127 ...). Yuvarlak bir kutunun gölgesi köşelerin ortalama yarıçapını kullanıyor. `inset` gölgeler sonra |
| `opacity` | Eleman tek bir grup olarak birleşiyor (`opacity` katmanı) ve `z-index: 0`'lı konumlandırılmış bir eleman gibi sıralanıyor (CSS Color 4 §9) |
| — | Referans sayfasının ilk hâlinde bulanık gölgenin uzun kuyruğu Chrome görüntüsünde 1000 pikselden fazla `[254,254,255]` bıraktı; tolerans muhafızı bunu beyazla aynı sayılabilecek bir "düz renk" olarak yakaladı. Gölge kısaltıldı |
| — | **WPT'de 29 rastlantı ortaya çıktı.** Kenarlıklar ve opaklık boyanınca daha önce boş karede eşleşen testler düştü: 17'si yüklenmeyen bir görüntüye ya da boyutlandırılmayan bir yerine konan elemana (`<img>`, `<iframe>`) dayanıyor, 7'si inline içinde blok (Erk bloğu satır içi içerik gibi diziyor; inline'ın kenarlığı bunu görünür kıldı), 2'si betik, 1'i `fieldset`, 2'si gerçek flex farkı (`column wrap`'te germe, `inline-flex`'te yüzdelik dolgu). Hepsi taban çizgisinde gerekçesiyle. Boş karede geçen test sayısı normal-flow'da 6'dan 0'a, css-position'da 6'dan 4'e, flexbox'ta 20'den 13'e indi |
| — | Bilinen sınır görünür oldu: satır sonunda başlayan kenarlıklı bir satır içi eleman, sol kenarını ve dolgusunu önceki satırda bırakıp metnini alt satıra taşıyor; Parley her satır içi kutudan sonra kırabiliyor. `borders` sayfasının skoru bu farkı taşıyor. Çözümü satır kırmanın içine girmeyi istiyor; açık |

Mutasyonlar (8/8 yakalandı): kenarlık kutunun tamamını dolduruyor; her kenara
tek renk; yarıçaplar küçültülmüyor; gölge kutunun içine de düşüyor;
bulanıklık parametresinde √2 yok (Chrome referans testi yakaladı); opaklık yok
sayılıyor; satır içi yan kenarlıklar her satırda; tuval elemanı kenarlığını
kaybediyor. Sağlamlık üretecine uç kenarlık, köşe, gölge ve opaklık değerleri
eklendi.

**İnceleme raporu: `docs/reviews/borders_commit_review.md` (2026-10-01, #19).**

| Rapor ne diyordu | Karar |
|---|---|
| `rounded_rect`'te köşe yarıçaplarının toplamı kutudan büyükse orantılı küçültme görünmüyor; Bézier kontrol noktaları dışarı taşabilir | **Geçersiz:** küçültme `display.rs`'te `corner_radii`'de, yarıçaplar display list'e girmeden yapılıyor (CSS Backgrounds 3 §5.5: tek bir ortak oranla, kenar başına komşu iki yarıçapın toplamı kenarı aşmayacak kadar). `paint.rs` hep küçültülmüş yarıçaplar alıyor. Testi `overlapping_radii_are_scaled_down_together`; sağlamlık üretecinde `border-radius: 1e30px` de var |

**M1.6, görüntüler, kaynak API'si ve lisans kapısı (2026-10-01).** M1.6
bununla bitti. Yeni `images` referans sayfası %98,12; kutuların 12/12'si
Chrome'la 1 px içinde. Diğer sayfaların skorları değişmedi.

| Plan ne diyordu | Gerçek |
|---|---|
| Görüntüler, sözleşmenin kaynak API'siyle (tür + MIME) | Renderer iş parçacığı belgenin adlandırdığı her URL'yi (`<img src>`, `background-image`'ın `url()` katmanları) bir kez `FromRenderer::Resources` ile ister, kareden **önce**: hemen yanıt veren host'un yanıtları, onsuz çizilen kareyi görmeden kuyruğa girer. Yanıt `ToRenderer::Resource { id, mime, data }` ya da `ResourceMissing { id }`. Eksik kaynak beklenen karelerde `Frame::resources_pending()` doğru; kabuğun ekran görüntüsü kipi bunu bekliyor (yoksa ilk, görüntüsüz kare yazılıyordu). Kaynaklar yeniden boyutlandırmada saklanıyor, `Load`'da düşüyor. İş parçacıksız yol için `render_html_with_resources(html, w, h, provide)` |
| Yanlış türde yanıt reddedilir (test) | MIME `image/png` ya da `image/jpeg` olmalı ve içerikle (imza baytları) uyuşmalı; MIME boşsa içerik karar verir (p1-contract §6). Stil sayfası MIME'ıyla gelen PNG, ya da JPEG etiketli PNG reddediliyor |
| png, jpeg | PNG `png` crate'iyle, JPEG `zune-jpeg` ile (Rust, `unsafe`'siz çekirdek yolu; `image` crate'i onlarca biçim getirirdi). Pikseller önceden çarpılmış RGBA. GIF, WebP, SVG, `data:` URL sonra |
| — | **Boyut bütçesi:** bir görüntü en çok 16384 px kenarlı ve 2²⁵ pikselli (8K ekran ve biraz fazlası, RGBA'da 128 MiB). Bütçe başlıktan, pikseller ayrılmadan önce denetleniyor. Testin ilk hâli PNG yolunun 16000 × 16000 iddia eden 59 baytlık bir dosya için önce bir gigabaytlık tamponu ayırdığını gösterdi; JPEG yolu çözmeyi bitirip sonra reddediyordu (68 s) |
| — | **Bir istek bir kez yanıtlanır:** ikinci yanıt ya da hiç yapılmamış bir isteğin yanıtı yok sayılıyor. **Kimlikler belgeler arasında sürüyor:** ilk hâlinde `Load` sayacı sıfırlıyordu ve önceki belgenin geç gelen yanıtı yeni belgenin aynı numaralı isteğine yazılıyordu. İkisi de testle bulundu |
| — | Sözleşme ikinci yanıtı tanımlamıyordu; karar: yok sayılır. Bir kaynağı sonradan değiştirmek (sıcak yeniden yükleme) gerekirse ayrı bir mesaj olur, M3'te API ile birlikte düşünülür |
| `<img>` | Yerine konan eleman: doğal boyut, `width`/`height` öznitelikleri (sunumsal ipucu, piksel ya da yüzde) ve CSS boyutları CSS 2 §10.4 kısıt tablosuyla (`replaced_size`: min/max ve oran birlikte). Satır içi `<img>` bir atom, taban çizgisinde duruyor. Blok akışında `auto` genişlik doğal genişlik ya da yükseklik × oran; Taffy'nin blok yerleşimi onu kapsayıcının genişliğine geriyordu (WPT yakaladı). Gelmeyen görüntü 0 × 0 |
| `background-image` | Konumlandırma alanı dolgu kutusu; `background-size` (`cover`, `contain`, uzunluk, yüzde, `auto`), `background-position`, `background-repeat` (`no-repeat` dışındaki her değer döşüyor; `space` ve `round` sonra), kenarlık kutusuna ve yuvarlak köşelere kırpılarak; birden çok katman, ilk katman üstte. Boyama vello'nun görüntü fırçasıyla, döşeme `Extend::Repeat` |
| Demo kabuğun sağlayıcısı: açılan dosyanın dizini ve `memory://` | Göreli URL'ler sayfanın dizininden. Şema (`file:`, `http:`), mutlak yol (dizinin içini gösterse bile) ve dizinden `..` ile çıkan yol reddediliyor; çözülmüş yolun dizinin içinde kaldığı denetleniyor. `memory://` tanınıyor ama kabuk henüz hiçbir varlık gömmüyor, boş dönüyor |
| — | `erk-wpt` de görüntüleri sayfanın dizininden veriyor. **WPT'de büyük sıçrama:** normal-flow 309 → 422, flexbox 534 → 568. Yedi rastlantı ortaya çıktı (dördü tablo, ikisi `white-space: pre`, biri belirli çapraz boyutta yüzde); taban çizgisinde gerekçeli |
| **Lisans kapısı** | `deny.toml`: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2/3-Clause, ISC, Unicode-3.0, Zlib, Unlicense, 0BSD, MPL-2.0 (Stylo), OFL-1.1 (gömülü fontlar), her biri gerekçesiyle. GPL ailesi yok: `r-efi`'nin "MIT OR Apache-2.0 OR LGPL-2.1-or-later" ifadesinden MIT seçiliyor. CI `licenses` job'ı `cargo deny --all-features --locked check licenses` |
| — | **Listeden başka yollar da kapalı:** `check-license-config.sh` `deny.toml`'da istisna (`exceptions`), açıklama (`clarify`), `private`/`ignore`, `skip`, `exclude`, hedef ya da özellik daraltması ve GPL ailesinden bir izin bulursa düşüyor; `all-features = true` zorunlu. Böyle bir karar betiği aynı PR'da değiştirmeyi istiyor |
| — | Renderer yüzey muhafızı satır satır okuyordu: rustfmt'nin çok satıra böldüğü `pub use` listesine eklenen bir tip görünmüyordu. Artık her `pub` öğesi `;` ya da `{`'ye kadar bütün okunuyor |
| — | **CI'da WPT kontrolü hiç test bulamadı** (2009 test "missing"). PR `Cargo.lock`'u değiştirdiği için rust-cache tam eşleşme bulamayıp `main`'in önbelleğine düştü; bu durumda geri yüklemeden önce `target/`'ı temizliyor ve `target/wpt`'deki WPT dosyalarını da sildi (dizinler kaldı, dosyalar gitti). Önceki PR'lar kilidi değiştirmediği için görülmedi. CI'daki WPT kopyası artık `target/` dışında (`wpt-checkout`) |

Muhafızlar kasıtlı ihlallerle denendi: izin listesinden MPL-2.0'ı çıkarmak
(28 crate reddedildi); `[[licenses.exceptions]]`, `exceptions = [...]`,
`[licenses.private]`, `ignore = true`, `[[licenses.clarify]]`, satır içi
tabloda `skip` ve `exceptions`, tırnaklı anahtar, `targets`, `exclude`,
`LGPL-2.1-or-later`, `MIT OR GPL-2.0`, `AGPL-3.0`, `all-features = false`,
eksik `deny.toml` (hepsi yakalandı; yorum satırı yakalanmıyor, doğru). Yüzey
muhafızı: çok satırlı `pub use` listesine kaçırılan bir tip (eski betik
geçiriyordu).

Mutasyonlar (15/15 yakalandı): konumlandırma alanı kenarlık kutusu;
`no-repeat` yok sayılıyor; `cover` küçük ölçeği alıyor; konum yok sayılıyor;
doğal oran stile girmiyor (ilk turda **geçti**: oran yalnızca blok akışındaki
yolda okunuyordu; `display: block; height: 10px` durumu eklendi); MIME yok
sayılıyor; `resources_pending` hep yanlış; `width` özniteliği yok sayılıyor;
döşeme `Pad` ile boyanıyor; PNG ve JPEG başlık bütçesi ayrı ayrı kaldırılıyor;
kabuk sağlayıcısında dizin denetimi ve şema denetimi ayrı ayrı kaldırılıyor
(şema denetimi ilk turda **geçti**: dizin denetimi listedeki her URL'yi zaten
reddediyordu; dizinin içini gösteren mutlak yol eklendi); `Load`'da kimlik
sayacı sıfırlanıyor; ikinci yanıt kabul ediliyor. Sağlamlık üretecine
gelen ve gelmeyen görüntüler, uç `background-size`/`background-position`
değerleri ve görüntülü öğeler eklendi.

**İnceleme raporu: `docs/reviews/images_commit_review.md` (2026-10-01, #20).**

| Rapor ne diyordu | Karar |
|---|---|
| Ele alınacak bir hata ya da öneri yok | Değerlendirilecek madde yok |
| Boyut sınırı `MAX_SIDE = 8000` | **Yanlış aktarım:** kodda kenar sınırı 16384 piksel, ayrıca görüntü başına 2²⁵ piksellik bir bütçe var; ikisi de başlıktan, pikseller ayrılmadan önce denetleniyor. Kodda değişiklik gerekmedi |

**M1.7a, `text-transform` ve HiDPI (2026-10-01).** M1.7 ikiye bölündü: bu
PR `text-transform` ve cihaz ölçeği; sistem fontları, `font-family` ve
fallback ayrı PR'da (sözleşmeye yazılacak bir karar istiyor: taramayı kim
yapar). İki yeni referans sayfası: `text-transform` %95,74 (9/9 kutu),
`hidpi` %96,19 (8/8 kutu, ölçek 2). Diğer skorlar değişmedi; WPT değişmedi.

| Plan ne diyordu | Gerçek |
|---|---|
| `text-transform`, `icu_casemap` ile | `icu_casemap` 2.3 (ağaçtaki ICU4X sürümü; yalnızca `icu_casemap` ve `icu_casemap_data` indirildi, Unicode-3.0). `uppercase`, `lowercase` ve `capitalize`, metnin diliyle: Türkçe/Azerice `i → İ`, `I → ı`; Yunancada büyük harf vurgusuz; Felemenkçede `ij → IJ`; Almanca `ß → SS` |
| Elemanın `lang`'ı | En yakın `lang` özniteliği (HTML §3.2.6.2), metin düğümü başına ve yalnızca metin dönüştürülüyorsa aranıyor. Geçersiz bir etiket bilinmeyen dil. **Dili olmayan metin kök kurallarla** dönüşüyor: Chrome orada tarayıcının kendi diline düşüyor (bu makinede Türkçe), Erk makinenin dilini okumuyor; bu yüzden referans sayfasında her metnin dili açıkça yazılı |
| — | Dönüşüm paragraf kurulurken, beyaz boşluk daraltılmadan önce yapılıyor (büyük/küçük harf eşlemesi boşluk üretmez ya da silmez). `capitalize` kelime sınırlarını öğe sınırlarında kesmiyor: `<span>a</span><b>b</b>` tek kelime, `Ab` |
| — | `capitalize`'ın kelimeleri Unicode kelime sınırlarına yakın: harf, işaret, rakam ve birleştirici; harfler arasındaki kesme işareti ya da nokta kelimeyi bölmüyor (`don't`), tire bölüyor (`foo-bar` → `Foo-Bar`); rakamla başlayan kelime değişmiyor (`3rd`) |
| — | **Chrome'dan bilinçli fark:** Chrome `capitalize`'da dili yok sayıyor, karakter başına başlık harfi alıyor: `lang="tr"` ile "ilk iş" Chrome'da "Ilk Iş". Erk "İlk İş" yazıyor (CSS Text 3 dile duyarlı eşlemeyi öneriyor; Türkçe bu adımın gerekçesi). Referans sayfasının skoru bu farkı taşıyor |
| HiDPI: cihaz ölçeği, layout CSS pikselinde, boyama cihaz pikselinde | Yeni `ToRenderer::Scale { factor }` mesajı ve `render_html_at_scale`. `Resize` cihaz pikseli; viewport `boyut / ölçek` CSS pikseli. Display list CSS pikselinde kalıyor; boyama tek bir `scale` dönüşümüyle, glifler, gölge bulanıklığı ve görüntüler dahil cihaz çözünürlüğünde. Stylo'nun cihazı ölçeği biliyor: `resolution` medya sorguları çalışıyor. Kabuk pencerenin ölçeğini açılışta ve `ScaleFactorChanged`'de gönderiyor. 1/64 ile 64 dışındaki ya da sayı olmayan ölçek 1 sayılıyor |
| — | Konumlar CSS pikseline yuvarlanıyor; ölçek 2'de bu cihaz pikseline de denk düşüyor, 1,5'te düşmüyor (kenarlar yarım cihaz pikselinde yumuşak). Chrome cihaz pikseline yuvarlıyor; kesirli ölçekte kenar keskinliği açık |
| — | **Referans testi ölçek öğrendi:** sayfa `<meta name="erk-device-scale" content="2">` ile ölçek seçiyor; Chrome `--force-device-scale-factor` ile yakalıyor, Erk aynı ölçekte çiziyor. Tolerans muhafızının "düz renk" eşiği (1000 piksel) CSS pikseli alanı olarak ölçekle büyüyor: ölçek 2'de yumuşak gölgenin her tonu dört kat piksel kaplayıp düz renk sayılıyordu. Aynı muhafız sayfanın ilk hâlinde gerçek bir hatayı da yakaladı: kartın beyazıyla sayfanın `#f8fafc`'si 7 farklıydı, kart çizilmese fark edilmezdi; zemin `#e2e8f0` oldu |

Mutasyonlar (14/14 yakalandı): dil yok sayılıyor; `lang` atalardan
aranmıyor; öğeler arası kelime bağlamı yok; kesme işareti kelimeyi bölüyor;
başlık harfinden sonrası küçültülüyor; baştaki harf olmayanlar atlanıyor;
kelime harf harf başlık harfine çevriliyor (Felemenkçe `IJ`); büyük harf dili
yok sayıyor; boyama ölçeği yok sayıyor; viewport ölçeğe bölünmüyor; ölçek
aralığı denetlenmiyor; iş parçacığı `Scale`'i yok sayıyor; Stylo ölçeği yok
sayıyor; Stylo'nun viewport'u ölçekleniyor. Sağlamlık üretecine
`text-transform`, uzunluğu değişen harfler ve tuhaf `lang` değerleri eklendi;
üretilen sayfalar 1, 1,5 ve 2 ölçeklerinde sırayla çiziliyor. Windows yayın
ikilisi 94 KB büyüdü (%0,6).

**M1.7b, sistem fontları (2026-10-02).** M1.7 bununla bitti. #21 için
inceleme raporu gelmedi. Karar p1-contract §6.2'de: tarama host'ta,
çekirdek bir katalog ve istediği yüzlerin baytlarını alıyor.

| Plan ne diyordu | Gerçek |
|---|---|
| fontique'in sistem taraması host'ta | Kullanıcıya soruldu (2026-10-02), plandaki yol seçildi: fontique `system` özelliği yalnızca `erk-shell`'de; Windows'ta DirectWrite, Linux'ta fontconfig (`fontconfig-dlopen`: bağlama zamanında C kütüphanesi yok), macOS'te CoreText. 13 yeni crate (9,1 MB'ı `windows` 0.62), hepsi izin listesinde. Reddedilen: saf Rust ile font dizinlerini taramak; yedek listeleri bizim tablolarımız olurdu |
| Font koleksiyonu çekirdeğe veri olarak | **İlk tasarım yüzleri katalogda gönderiyordu ve açılışı 1,3 s geciktirdi:** fontique bir ailenin yüzlerini dosyalarını açarak listeliyor (200 aile, 399 yüz). Katalog artık yalnızca aile adları, generic eşlemeler ve yazı sistemi yedekleri (12 ms); çekirdek yüzü `font:<aile>?weight=..&style=..` ile istiyor, en yakın yüzü host seçiyor. Ekran görüntüsü kipinde sayfa başına toplam süre 0,7–0,9 s |
| — | Çekirdek istemeden önce Parley'nin seçeceğini hesaplıyor: her metin için `font-family` listesinde katalogda bulunan ilk aile; gömülü Noto Sans'ın çizemediği her karakter için yazı sisteminin (ICU) yedek listesi, katalogda o dil için liste varsa onunla; emoji için `emoji` generic ailesi. Gelen yüzler fontique'e kaydediliyor, generic aileler ve yedekler katalogdan kuruluyor; gömülü font her listenin sonunda |
| — | **Dil yalnızca listesi olan diller için Parley'ye söyleniyor:** fontique'in izlediği bir dil (Yidiş, Farsça, Japonca) listesiz kalırsa hiçbir yedek bulunmuyor. Han karakterleri için fontique her sorguya dilsiz Han listesini kendisi ekliyor; bu yüzden bu kuralın testi İbranice (`lang="yi"`) |
| — | `font-style: italic` artık yüz seçiyor (host'un fontlarıyla; gömülü fontta italik yok, dik çiziliyor) |
| Testler yine gömülü fontla | Katalog yoksa her aile Noto Sans: altın görüntüler ve Chrome referansı değişmedi. Font testleri için `tests/fonts/ErkTest.ttf` (896 bayt, fontTools ile üretildi, üreteci yanında): `x`, `中`, `文`, `א`, `😀`, her biri dolu bir kare; hangi fontun çizdiği piksellerden okunuyor |
| — | **Muhafız:** `check-core-io.sh` artık çekirdeğin bağımlılıklarında fontique ya da Parley `system` özelliğini (`cargo tree -p erk-renderer -e features`) ve kaynakta `load_system_fonts`, `load_fonts_from_paths`, `system_fonts: true`'yu reddediyor. Kasıtlı ihlaller: Parley'ye `system`, fontique'i doğrudan `system` ile eklemek, üç kaynak çağrısı (5/5 yakalandı; Python'un `bash`'inde `cargo` bulunamayınca betik geçmedi, doğru) |
| — | **Bulunan sınır:** sağdan sola bir paragraf yönünü `direction`'dan değil ilk güçlü karakterinden alıyor; soldan sağa bir kutudaki İbranice sağdan başlıyor. Parley 0.11'de taban yönü ayarı yok (`None` sabit). css-support'ta "Later" |
| — | Satır yükseklikleri ve `vertical-align` hâlâ Noto Sans'ın ölçüleriyle; sistem fontuyla `line-height: normal` Chrome'dan birkaç piksel farklı olabilir. Açık |

Mutasyonlar (çekirdek 14/14, host 3/3 yakalandı): CSS aile listesi yok
sayılıyor; gelen yüzler kaydedilmiyor; generic aileler kurulmuyor; yedekler
kurulmuyor; dil hiç söylenmiyor; dil hep söyleniyor (ilk turda **geçti**:
fontique Han için dilsiz listeyi kendisi ekliyor; İbranice testi eklendi);
yedek yüzler istenmiyor; gömülü fontun kapsamı yok sayılıyor; dil listesi
istekte yok sayılıyor; font MIME'ı yok sayılıyor; aile adı kodlanmıyor;
italik URL'de yok; fontlar beklenmiyor; aile adı büyük-küçük harfe duyarlı;
host'ta fazladan sorgu anahtarı kabul ediliyor; host her türe font veriyor;
host stili yok sayıyor (ilk turda **geçti**: en yakın yüz testi eklendi).
İlk turda geçen bir mutasyon daha vardı: font baytlarının çözülüp
çözülmediği hiçbir yerde görünmüyordu (fontique zaten kaydetmiyor, metin
yedekle çiziliyor); denetim kaldırıldı, yalnızca MIME denetimi kaldı.
Windows yayın ikilisi 810 KB büyüdü (%5,3, DirectWrite kodu).

**İnceleme raporları: `docs/reviews/hidpi_commit_review.md` (#21) ve
`docs/reviews/system_fonts_commit_review.md` (#22), 2026-10-02.**

| Rapor ne diyordu | Karar |
|---|---|
| hidpi: düzeltilecek bir şey yok; önceki rapordaki 8000 düzeltmesi not edilmiş | Değerlendirilecek madde yok |
| system_fonts: host'taki yüzde kod çözmede `%25`'in en son çözülmesi çift çözmeyi önlüyor | **Geçerli, ama korunmuyordu:** test (`A%26B %231 %3F%25`) `%25` önce çözülse de geçiyordu. Adında `%26` geçen bir aile (`100%26`) eklendi; sıra bozulunca test "100&" görüp kırılıyor (mutasyon yakaladı) |
| system_fonts: çekirdeğin G/Ç yapmaması, muhafız, tembel katalog, Noto Sans son yedek | Tespit; yapılacak bir şey yok |
| system_fonts: Parley güncellenince CSS `direction` desteği | Geçerli, zaten kayıtlı: css-support'ta "Later" satırı ve ayrı bir görev önerisi |

**M1 kabulü (2026-10-02).** M1 bununla bitti.

| Plan ne diyordu | Gerçek |
|---|---|
| Ayarlar ekranı maketi Chrome referans testinde | `examples/settings.html`: yan menü, kartlar, anahtarlar ve seçim kutuları (form denetimleri M5'e kadar kutularla), büyük harfli bölüm başlıkları, rozet, soluk bir satır, düğmeler. `merhaba` gibi örneklerden okunan bir referans sayfası. %97,40, kutuların 48/48'i Chrome'la 1 px içinde. Görünen tek fark: satır içi elemanın arka planı köşe yarıçapı almıyor, rozet Chrome'da hap, Erk'te kare |
| — | **Maket iki gerçek hata buldu.** (1) Satırı küçük puntolu bir span'la biten metin satırı kısaltıyordu ("Bildirimler" + rozet: 35 yerine 31 px): Parley 0.11 satıra son koşunun yüksekliğini veriyor, daha küçükse de. Her satır kutusu bloğun fontunun strut'ıyla başlar (CSS 2 §10.8.1); stil aralıkları olan paragraflarda satır yüksekliği artık strut'ın altına inmiyor. (2) İlk yakalamada tolerans muhafızı iki tasarım hatasını gösterdi: kartların 1 px'lik gölgesi kenar boyunca 1000 pikselden uzun düz renk satırları bırakıyordu (gölge kaldırıldı) ve seçim kutusunun `#f8fafc` zemini beyaz karttan yalnızca 7 farklıydı, zemin çizilmese fark edilmezdi (zemin kaldırıldı) |
| Akış layout'u sınıflaması M1.6'dan sonra tekrarlanır | Tekrarlanırken üçüncü bir hata çıktı: genişliği ve yüksekliği verilen bir `<img>` yüksekliğini yok sayıyordu (40 × 1 istenen 4 × 2'lik görüntü 40 × 20). Taffy 0.14'ün yaprak layout'u en-boy oranı olan bir kutuyu en az genişlik / oran yüksekliğinde tutuyor; Erk boyutu `replaced_size` ile zaten çözdüğü için oranı Taffy'ye ikinci kez vermiyor. **WPT'de 87 test geçti** (normal-flow 422 → 508, flexbox 568 → 569), gerileme yok; hepsi bu düzeltmeden (strut düzeltmesi tek başına hiçbir WPT sonucunu değiştirmedi) |
| **Akış layout'u kararı** | **Kesin: Taffy'de kalınır.** normal-flow'da düşen 238 testin 205'i Erk'in desteklemediği ya da planlamadığı bir şeyi kullanıyor: dış stil sayfası (54), tablo (51), betik (50), Erk'in çizmediği bir yerine konan eleman (30), float (8), yazı yönü (5) ve birkaç başka. Kalan 33'ün 11'i inline içinde blok (Erk'in kendi inline layout'u), 7'si yüzdelik ya da miras boyut, 6'sı yerine konan eleman boyutu, 4'ü inline-block boyama sırası, 5'i diğer. Hiçbiri Taffy'nin block layout'unu bütünüyle değiştirmeyi gerektirmiyor |
| `css/css-text` taban çizgisi | Kullanıcı izniyle (3017 dosya, 4,2 MB) `tests/wpt/dirs.txt`'e eklendi; ek destek dizini gerekmedi. 1489 reftest'ten 608'i geçiyor (%40,8; 15'i boş karede). Geçenler: i18n 158/158, text-transform 91/108, shaping 26/28. En büyük açık white-space (45/422: `pre` ve benzeri yok); 718 test Ahem fontunu `@font-face` ile istiyor |
| Fuzz job'ı | `fuzz/`: kendi workspace'i olan `erk-fuzz` crate'i, `render_html` hedefi; girdi UTF-8'e kayıpla çevrilip renderer iş parçacığının 16 MiB yığınlı bir iş parçacığında çiziliyor. CI `fuzz` job'ı sabit `nightly-2026-09-25` ve `cargo-fuzz 0.13.2` ile beş dakika koşuyor; tohum: sağlamlık korpusu ve referans sayfaları; çöken girdi artifact olarak saklanıyor. Kilit dosyası ana kilitten kopyalandı (renderer'ın bağımlılıkları aynı sürümde); `cargo fetch --locked` güncelliğini denetliyor. Yeni crate'ler: `libfuzzer-sys` 0.4.13 (izinle) ve onun zorunlu bağımlılığı `arbitrary`. libFuzzer'ın lisansı NCSA izin listesine gerekçesiyle girdi; lisans kapısı `fuzz/`'u da denetliyor. Yerelde nightly yok: hedef kararlı araç zinciriyle derlendi (libFuzzer MSVC ile derlendi), ilk gerçek koşu bu PR'ın CI'ında. **Doğrulanmadı:** job'ın bir çökmeyi yakaladığı, kırıldığı ve girdiyi artifact olarak sakladığı henüz kasıtlı bir çökmeyle denenmedi; M1 kabulünün açık maddesi |
| — | **İlk CI koşusu kırmızıydı, bir sızıntı yüzünden:** LeakSanitizer korpusun ilk sayfasında 6 parça, 40.800 bayt bildirdi (exit 77); job girdiyi `fuzz/artifacts/` altına yazdı ve artifact yüklendi (571 bayt). Bu, kırılma ve saklama akışını **bir sızıntı için** gösterdi; panik yolu hâlâ denenmedi. Yerel ölçüm: aynı sayfa tek bir iş parçacığında 1000 kez çizilince süreç belleği 6,07'den 6,41 MB'a çıkıyor (büyüme yok sayılacak kadar az), her çizim için yeni bir iş parçacığında ise 7,57'den 20,43 MB'a (iş parçacığı başına ~14 KB). Sızıntı bir bağımlılığın iş parçacığı yerel verisi: iş parçacığı bitince geri verilmiyor. Kabuğun tek bir renderer iş parçacığı olduğu için pencere büyümüyor; fuzz hedefi artık kabuk gibi tek ve kalıcı bir iş parçacığında çiziyor (çizim başına bir sızıntıyı yine yakalar). **Açık (M3):** uygulamayı tekrar tekrar kurup yıkan bir host her seferinde ~14 KB kaybeder; hangi bağımlılık olduğu bulunmalı |
| Bellek ve ilk kare M1.0'la karşılaştırılır | Aşağıdaki tablo; yöntem M1.0'ınki |

| Ölçüm | M1.0 | M1 sonu |
|---|---|---|
| `erk.exe`, varsayılan yayın profili | 14.899.200 bayt | 16.211.968 bayt (+%8,8: görüntü çözücüleri, ICU casemap, DirectWrite) |
| `nodes-1000.html`, `render_html` ilk çağrı / 30 çağrının medyanı | 116,85 / 70,95 ms | 120,26 / 71,21 ms |
| `merhaba.html`, ilk çağrı / medyan | 8,32 / 6,88 ms | 9,38 / 6,35 ms |
| `settings.html`, ilk çağrı / medyan | — | 15,87 / 12,44 ms |
| `erk nodes-1000.html` penceresi, ilk kareden 3 sn sonra: özel bellek / çalışma kümesi / tepe | 13,0 / 30,1 / 40,3 MB | 14,2 / 32,1 / 42,4 MB |
| `erk merhaba.html` penceresi, aynı ölçüm | 12,6 / 29,6 / 35,5 MB | 12,8 / 31,0 / 37,7 MB |
| `erk settings.html` penceresi, aynı ölçüm | — | 13,3 / 31,6 / 38,5 MB |

Okuma: M1'in getirdiği her şeye (satır içi layout, flexbox, konumlandırma,
kenarlıklar, gölgeler, görüntüler, sistem fontları) rağmen tam yeniden
çizim süresi değişmedi; bellekteki ~1 MB'lık artış fontique'in sistem
koleksiyonu. 1000 elemanlı bir sayfanın tam yeniden çizimi hâlâ ~71 ms:
M5'in artımlı işinin gerekçesi aynen duruyor.

Mutasyonlar (3/3 yakalandı): `%25` önce çözülüyor; strut alt sınırı
kaldırılıyor; oran Taffy'ye yine veriliyor.

**Fuzz job'ının kasıtlı ihlalle denenmesi (2026-10-02, taslak PR #24,
birleştirilmeden kapatıldı, dal silindi).**

| Deneme | Sonuç |
|---|---|
| `render_html` `Z` ile başlayan bir sayfada panikliyor | **Job yeşil geçti: panik hiç bulunamadı.** Boru hattı değil hız: hedef saniyede 6–9 girdi deniyor (ASan, 55 KB'a varan tohumlar), beş dakikada ~2–3 bin; ~1300 baytlık bir girdinin ilk baytının `Z` olması bu kadar denemede pek olası değil. Bu, job'ın bugünkü haliyle zayıf bir kapı olduğunu gösterdi |
| Fuzz hedefinin render iş parçacığı, uzunluğu 7'ye bölümünden 3 kalan her girdide panikliyor | **Job kırıldı (1 dk 19 sn):** `fuzz canary: length 661`, libFuzzer "deadly signal", girdi `crash-947da3ff…` olarak yazıldı ve `fuzz-artifacts` artifact'ı yüklendi (654 bayt). Render iş parçacığındaki bir panik libfuzzer-sys'in panik kancasıyla süreci durduruyor; yakalama ve saklama akışı panik için doğrulandı (koşu 37002635739) |

**Fuzz hızı (2026-10-02, PR #26).** Saniyede 6–9 girdi kolay bir paniği
bile bulamıyordu. Üç ayar ikişer dakika ölçüldü (libFuzzer'ın son
istatistikleri):

| Ayar | Saniyede girdi | 2 dakikada deneme | Yeni birim | Bellek tepe |
|---|---|---|---|---|
| ASan, `max_len` 65536 (o güne kadarki) | 9 | 1.124 | 717 | 546 MB |
| ASan, `max_len` 4096 | 10 | 1.285 | 742 | 525 MB |
| Sanitizer yok, `max_len` 4096 | 67 | 8.128 | 2.128 | 102 MB |

Girdi boyutu değil AddressSanitizer yavaşlatıyor. Yerel ölçüm de bunu
söylüyor: 320 × 240'ta tek bir `render_html` çağrısının sabit maliyeti
0,78 ms (ayrıştırma ve stil 0,4 ms; 800 × 600'de 4,15 ms, yani sabit
maliyetin çoğu piksel sayısıyla orantılı: zemini boyamak ve kareyi
kopyalamak). ASan'ı kaldırmanın bedeli LeakSanitizer'ı ve bağımlılıkların
`unsafe` kodundaki bellek hatası denetimini kaybetmek olurdu; kullanıcı
kararıyla iki job paralel koşuyor: `fuzz (none)` sanitizer'sız (67/sn,
panikler için), `fuzz (address)` ASan'la (9/sn, sızıntı ve bellek hataları
için). CI süresi aynı kaldı, CI dakikası iki katı. Sanitizer'sız job yeni
bir muhafız olduğu için kasıtlı bir panikle ayrıca denendi (aşağıda).

**İnceleme raporu: `docs/reviews/acceptance_commit_review.md` (#23,
2026-10-02).**

| Rapor ne diyordu | Karar |
|---|---|
| Uzun ömürlü render iş parçacığı, bağımlılıkların iş parçacığı yerel sızıntılarının "önüne geçmiş" | **Kısmen:** düzeltme sızıntıdan kaçınıyor, onu gidermiyor. Hangi bağımlılığın iş parçacığı başına ~14 KB bıraktığı M3 için açık; uygulamayı tekrar tekrar kuran bir host bunu görür |
| "Tüm CSS2 layout gereksinimlerinin son ve en zorlu edge-case senaryoları halledilmiş" | **Yanlış:** normal-flow'da 238 test düşüyor; 205'i desteklenmeyen ya da planlanmayan özellik, ama 33'ü açık durumlar (11'i inline içinde blok) |
| "M1 acceptance tamamlanabilir" | O commit'te fuzz job'ının bir paniği yakaladığı henüz denenmemişti; yukarıdaki denemeyle doğrulandı, M1 ancak şimdi bitti |
| Strut, oran ve kod çözme testleri | Tespit; yapılacak bir şey yok | Kural takviminin M1 satırları
(fuzz, boyut bütçesi, css-support'taki her "Supported" satırın testi) proje
kurallarında yürürlükteki kurallara taşındı; son ikisi M1.0'dan beri CI'da
zorlanıyordu.
