# Erk Engine M2 (Etkileşim temeli) Uygulama Planı

**Hedef:** Erk'te çizilen bir sayfa girdiye yanıt veriyor. Uzun bir sayfa
kayıyor, fare üstüne gelince `:hover` stili değişiyor, bir tıklama doğru
`NodeId`'yi host'a bildiriyor. Sayaç demosu çalışıyor: düğmeye basılınca
host'un (Rust) sayacı artıyor, sayının metni değişiyor, kare yeniden
çiziliyor; bunu otomatik bir test tıklama gönderip altın görüntüyle
doğruluyor. GPU yolu var, yoksa CPU'ya düşüyor. Tam yeniden hesaplamanın
kare süresi kayıtlı.

**Mimari:** M1'in hattı değişmez: html5ever → arena DOM → Stylo → Taffy +
Parley → display list → vello. M2'nin özgün işi **kalıcı belge**: bugün
renderer iş parçacığı her karede HTML'i yeniden ayrıştırıyor; M2'de belge bir
kez ayrıştırılır ve kareler arasında yaşar. Eleman durumu (`:hover`,
`:active`, `:focus`), kaydırma konumları ve host'un değişiklikleri o belgenin
üstünde tutulur. Her durum değişikliği **bilerek kaba** bir tam yeniden stil,
layout ve boyama ister; artımlı iş M5'te. M2'nin kare süresi ölçümleri bir
performans iddiası değil, M5'in kıyaslanacağı tabandır.

Kabuk ile renderer hâlâ yalnızca düz veri mesajlarla konuşur (proje
kuralları, `check-renderer-surface.sh`). Girdi kabuktan renderer'a, olaylar
renderer'dan kabuğa gider. M3'te DOM, stil ve layout UI iş parçacığına,
yalnızca raster ayrı iş parçacığına iner (p1-contract §1.1); M2 bunu
zorlaştıran bir şey eklemez: belge, durum ve hit-test tek bir yapıda
toplanır, iş parçacığı onu yalnızca taşır. Gerekçeler:
[p1-contract.md](../design/p1-contract.md), [p1-embedded.md](../design/p1-embedded.md).

**Teknoloji:** M1'deki sürümler (Stylo 0.20, Taffy 0.14, Parley 0.11.1,
vello_cpu 0.2, winit 0.30). Yeni: `vello_hybrid` ve wgpu (M2.5, indirme izni
o adımda sorulur).

## Genel kısıtlar

- **Sözleşme:** çekirdek G/Ç, ortam ve saat kullanmaz (`check-core-io.sh`).
  Olay türleri, koordinatlar ve `NodeId` sözleşmenin şeklindedir
  (p1-contract §2, §5, §10): olaylar mantıksal (CSS) pikselde, olay türleri
  `ERK_EVENT_*` sırasıyla, dağıtım capture, target ve bubble alt kümesi.
  M2'de sınırdan `NodeId` düz bir `u64` olarak geçer (iç temsil); uygulamaya
  özel karıştırma M3'te C-ABI ile gelir.
- **Kapsam:** css-support.md. Bir özellik matrise "Supported" olarak ancak
  testini adlandırarak girer. Form denetimleri (`<input>`, odak halkası,
  metin seçimi, IME) M5'te; M2'nin odağı tıklanabilir elemanlar ve klavyeyle
  odak gezinmesiyle sınırlı.
- **Adım başına PR:** her adım kendi dalında (`m2/...`) ve kendi PR'ında;
  PR'lar `main`'den açılır. Render'ı değiştiren her adım Chrome skor
  tablosunu commit gövdesine ve PR açıklamasına yazar; yeni render davranışı
  kendi referans sayfasıyla gelir.
- **Test disiplini:** her değişiklik testle başlar; her yeni test, koruduğu
  hatayı üreten bir mutasyonla denenir; her yeni muhafız kasıtlı bir ihlalle.
  Girdi testleri renderer'ı gerçek mesaj protokolüyle sürer, zaman aşımıyla
  bekler.
- **Belirleyicilik:** testler gömülü Noto Sans'la ve CPU yoluyla çizer; GPU
  yolu altın görüntülere girmez.

## Dosya yapısı (M2 sonunda)

```
crates/erk-renderer/src/page.rs         kalıcı belge, hit-test (boyama sırasının tersinden), tıklama (M2.0, M2.1)
crates/erk-renderer/src/events.rs       odak olayları (M2.2)
crates/erk-renderer/src/scroll.rs       kaydırma kapları ve kırpma (M2.3)
crates/erk-renderer/tests/input.rs      girdi protokolü testleri (M2.1–M2.3)
crates/erk-renderer/tests/counter.rs    sayaç demosunun otomatik testi (M2.4)
crates/erk-shell/src/gpu.rs             vello_hybrid yolu, CPU'ya düşme, render hedefi (M2.5)
examples/counter.html                   sayaç demosunun sayfası (M2.4)
examples/perf/long-page.html            kaydırma ve kare süresi ölçüm sayfası (M2.0)
```

---

### M2.0: Kalıcı belge, metin geometrisi testi, ölçüm tabanı

- [x] Renderer iş parçacığı belgeyi `Load`'da bir kez ayrıştırır ve saklar;
  kareler saklanan belgeden stil, layout ve boyama yapar. `NodeId`'ler kareler
  arasında aynı kalır (test). Görüntü ve font istekleri değişmez.
- [x] **Metin geometrisi testi** (M1'in açık bulgusu): kutu geometrisi testi
  satır içi metni atlıyor; `paragraphs` ve `inline-styles`'ın düşük piksel
  skoru elle bakılınca yerleşim değil glif çizimi çıktı (satırlar ve kelime
  kenarları Chrome'la 1 px içinde). Referans testine kalıcı olarak eklenir:
  her sayfada metin satırlarının dikey bantları ve kelime kenarları Chrome
  görüntüsüyle karşılaştırılır; 1 px'ten büyük fark testi kırar. M2'nin
  yeniden stil yolları metni bozarsa piksel skoru değil bu test yakalar.
- [x] Ölçüm tabanı: `examples/perf/long-page.html` (kaydırılacak uzun sayfa)
  ve `measure`'a kalıcı belgeden kare süresi (ayrıştırmasız). M1'in 800 × 600
  ölçümü sabit maliyetin çoğunun piksel sayısıyla orantılı olduğunu gösterdi
  (zemini boyamak ve kareyi kopyalamak); bu da ayrıca ölçülür.

### M2.1: Girdi hattı, hit-test, tıklama, denetim

- [x] Mesajlar (düz veri): `ToRenderer::Pointer { kind: Move | Down | Up |
  Leave, x, y, button, modifiers }` (mantıksal piksel). Kabuk winit'in fare
  olaylarını ölçekten bağımsız mantıksal piksele çevirip gönderir.
- [x] Hit-test: son layout'un kutuları boyama sırasının tersinden gezilir
  (yığın bağlamları, `z-index`, akış); metnin üstündeki nokta metnin
  elemanını verir; `pointer-events: none` atlanır; `visibility: hidden` hedef
  olmaz.
- [x] Tıklama: aynı eleman (ya da ortak ata) üstünde basma ve bırakma bir
  `ERK_EVENT_CLICK` olur; `FromRenderer::Event { kind, target, path, x, y,
  modifiers }`. `path` hedeften köke düğümler: capture, target ve bubble
  sırasını host bu yoldan kurar. Otomatik test: bir tıklama doğru `NodeId`'yi
  bildiriyor (kabulün maddesi).
- [x] Denetim (p1-contract §8.1): `ToRenderer::InspectAt { x, y }` hit-test'in
  `NodeId`'sini döndürür; `ToRenderer::Highlight { node }` seçili düğümün
  kutularını bir kaplamayla çizer. **Muhafız** (p1-contract §11): vurgu
  açıkken ve kapalıyken DOM dökümü ve hesaplanmış stiller aynı; display
  list'te yalnızca kaplama öğesi farklı.

### M2.2: Eleman durumu ve odak

- [x] `:hover`: imlecin altındaki eleman ve ataları; `:active`: basılı tuşun
  elemanı ve ataları; `:focus`, `:focus-within`. Durum erk-style'ın yan
  tablosundaki `ElementState`'e yazılır (bugün yalnızca bağlantılar için
  dolu) ve tam yeniden stil çalışır.
- [x] Odak: tıklama odaklanabilir elemanı (`button`, `a[href]`, `tabindex`)
  odaklar; Tab ve Shift+Tab belge sırasıyla gezer; odaktaki düğmede Enter ve
  Space bir tıklama üretir. `ERK_EVENT_FOCUS` ve `ERK_EVENT_BLUR`. Klavye
  mesajı: `ToRenderer::Key { key, state, modifiers }`.
- [x] Referans sayfası: `:hover` ve `:focus` stilli düğmeler; Chrome
  görüntüsü durumsuz, Erk testi durumu mesajla verip pikselleri doğrular.

### M2.3: Kaydırma, kırpma, imleç

- [x] `overflow: hidden | auto | scroll` çocukları kırpar (display list'e
  kırpma katmanı); bu M1'de açık kalan "çocuklar kırpılmıyor" sınırını da
  kapatır.
- [x] Kaydırma kapları ve kök görüntü alanı: `ToRenderer::Wheel { dx, dy,
  x, y }` imlecin altındaki en içteki kaydırılabilir kabı kaydırır, sınırda
  dışarı taşar; kaydırma konumu belgeyle saklanır. Hit-test kaydırmayı
  hesaba katar. Basit kaydırma çubukları (kaplama olarak).
- [x] `cursor` özelliği: `FromRenderer::Cursor(shape)`, kabuk winit'e verir.
- [x] Otomatik test: uzun bir sayfa tekerlekle kayıyor ve görünen içerik
  değişiyor (kabulün maddesi). Referans sayfası: kırpılan ve kaydırılmış kaplar.

### M2.4: İlk değişiklik ve sayaç demosu

- [x] M4'ün `Mutation` API'sinin ilk parçası: `ToRenderer::SetText { node,
  text }` (`erk_node_set_text`) ve `ToRenderer::Query { request, selector }`
  → `FromRenderer::QueryResult { request, node }` (`erk_query`, CSS seçici).
  Eski bir `NodeId` hata olarak döner, çökmez (test).
- [x] Sayaç demosu: `examples/counter.html` ve demo host (Rust, kabukta):
  düğmeye basılır, host'un sayacı artar, sayının metni değişir, kare yeniden
  çizilir. Otomatik test: tıklama gönderilir, sayının değiştiği kare altın
  görüntüyle doğrulanır (kabulün maddesi).

### M2.5: GPU yolu ve host'a çizim

- [ ] `vello_hybrid` (wgpu) ile pencere yüzeyine çizim; yüzey ya da adaptör
  yoksa `vello_cpu`'ya düşme (test: GPU kapalıyken kare yine geliyor).
  Bağımlılık ağacı büyük: indirme izni ve lisans kapısı bu adımda.
- [ ] **Host'a çizim:** render hedefi bir soyutlama olur; ya kabuğun kendi
  penceresi ya da host'un verdiği bir pencere (raw-window-handle,
  p1-contract §7'deki host'un döngüsü). M3'ün API'si bunun üstüne kurulur.
  Host'un Erk belgesinin içine kendi GPU çizimini yapması (surface) ayrı:
  M10.
- [ ] GPU ve CPU kare sürelerinin ölçümü; altın görüntüler ve Chrome
  referansı CPU'da kalır.
- [ ] p1-contract §8.2'nin açık sorusu (surface: host callback'i mi doku mu)
  bu adımın ölçümünden sonra yazılır.

### M2.6: Metin düzenini sağlamlaştırma

M1 sonunda metin ağırlıklı sayfaların piksel skoru düşük (paragraphs %48,27,
inline-styles %48,60, vertical-align %68,11, inline-boxes %75,10) ve
`css/css-text` %40,8. `paragraphs` ve `inline-styles`'ta elle bakılınca
satırlar ve kelime kenarları Chrome'la 1 px içinde çıktı; ama metin ağırlıklı
arayüzlerde görünür sorunlar kalmış olabilir. Bu adım farkları tek tek
inceler, gerçek olanları düzeltir.

- [ ] `inline-boxes`, `paragraphs`, `vertical-align` ve `inline-styles`
  sayfalarında M2.0'ın metin geometrisi testinin bulduğu her fark: glif
  çizimi mi (kabul, gerekçesiyle) yerleşim mi (düzeltilir).
- [ ] **Satır içi kutu parçalanması** (M1.6'nın bilinen sınırı, kullanıcının
  da gördüğü): satır sonuna sığmayıp alt satıra inen bir satır içi eleman
  açılış dolgusunu ya da kenarlığını ve önündeki boşluğu önceki satırda
  bırakıyor; arka planı orada ince bir dikey çizgi olarak görünüyor. Chrome
  satır sonundaki boşluğu kırpar ve elemanın açılış kenarını metniyle aynı
  satıra koyar. Test: arka planlı bir `<span>` satır sonunda kırılınca önceki
  satırda span'ın hiçbir pikseli kalmıyor.
- [ ] **`<br>` satırı kırmıyor** (M2.3'te bulundu): hiçbir kod `<br>`'yi
  ele almıyor, metin aynı satırda sürüyor. Zorunlu satır sonu olarak
  eklenir; test: `<br>` ile ayrılmış kelimeler ayrı satırlarda, Chrome'un
  metin geometrisiyle.
- [ ] `white-space`: `css-text/white-space` 45/422 geçiyor. `nowrap`, `pre`,
  `pre-wrap`, `pre-line` alt kümesinin bu adımda mı geleceği, testlerin
  sınıflamasıyla kararlaştırılır ve gerekçesiyle yazılır.
- [ ] Düzeltmelerin skoru ve WPT sonuçları commit gövdesinde.

### M2.7: CSS Position analizi

- [ ] `css/css-position` 48/251 geçiyor. Düşen 203 testin her biri
  sınıflanır: desteklenmeyen ya da planlanmayan bir özellik (sticky, tablo,
  yazı yönü, betik) mi, yoksa Erk'in desteklediği bir özellikte gerçek bir
  hata mı. Sınıflama test adı başına bu planın yürütme notlarına yazılır.
- [ ] Desteklenen özelliklerdeki hatalardan ucuz olanlar düzeltilir, kalanlar
  gerekçesiyle açık kalır.

### M2.8: macOS CI ve kabul

- [ ] CI'a macOS job'ı (derleme ve testler; fontique CoreText yolu).
- [ ] Kabul maddelerinin hepsi otomatik testle; tam yeniden hesaplamanın kare
  süresi kayıtlı; `roadmap.md`'de M2 "Bitti".

### M2 kabulü

- [ ] Uzun bir sayfa kayıyor, hover stili değiştiriyor, bir tık doğru
  `NodeId`'yi raporluyor (otomatik testler).
- [ ] Sayaç demosu çalışıyor; otomatik test tıklama gönderip sayının
  değiştiği kareyi altın görüntüyle doğruluyor.
- [ ] GPU yolu yoksa CPU'ya düşüyor (test).
- [ ] Tam yeniden hesaplamanın kare süresi kaydedilmiş, M5'in tabanı olarak.
- [ ] Metin geometrisi testi her referans sayfasında yeşil; satır içi kutu
  parçalanması düzeltilmiş (test).
- [ ] `css/css-position`'da düşen her test sınıflanmış.
- [ ] macOS CI yeşil.
- [ ] `roadmap.md`'de M2 "Bitti".

## Açık sorular

- **Olay aboneliği nerede?** p1-contract'ta host belirli düğümlere abone olur
  (`erk_on`). M2'de renderer her tıklamayı yoluyla bildirir, abone listesi
  kabukta (demo host'ta) tutulur; abonelik kaydının çekirdeğe girip girmeyeceği
  M3'te API ile kararlaştırılır. M2'yi bloke etmiyor.
- **Kare sıklığı:** her durum değişikliği bir kare ister; fare hareketinde
  saniyede düzinelerce. M2 mesaj kuyruğunu M0'daki gibi birleştirir (yalnızca
  son konum işlenir); bunun yeterli olup olmadığı M2.0'ın ölçümüyle görülür.

---

## Yürütme Notları

*(Adımlar yürütüldükçe, planın yanlış çıkan varsayımları ve doğrulanan
gerçeklerle doldurulur.)*

**İnceleme raporu: `docs/reviews/fuzz_commit_review.md` (#26, 2026-10-02).**

| Rapor ne diyordu | Karar |
|---|---|
| İki paralel fuzz job'ı, artifact ve önbellek adlarının ayrılması, kasıtlı panikle deneme | Tespit; yapılacak bir şey yok |
| "`max_len`'in hızlı modda 4096 tutulması fuzzer'ın verimini artıracaktır"; ileride yeniden ayarlanabilir | **Ölçülenle uyuşmuyor:** `max_len`'i 65536'dan 4096'ya indirmek ASan job'ını yalnızca saniyede 9'dan 10 girdiye çıkardı; 7 katlık hız sanitizer'ı kaldırmaktan geldi. Yeniden ayarlama notu yerinde: girdiler ağırlaşırsa ölçülerek |

**M2.0 (2026-10-05).** Kalıcı belge, metin geometrisi testi, ölçüm tabanı.

| Plan ne diyordu | Gerçek |
|---|---|
| Belge `Load`'da bir kez ayrıştırılır | `Page` (`page.rs`): belge `Load`'da ayrıştırılıyor, her kare ondan stil, layout ve boyama yapıyor. Test: aynı `Page`'in farklı boyut ve ölçeklerdeki kareleri, o boyutta baştan ayrıştırılmış bir sayfanınkiyle piksel piksel aynı |
| Metin geometrisi testi | Chrome'un `Range.getClientRects()`'i her metin düğümünün satır satır dikdörtgenini veriyor; yakalama aracı bunları sayfa başına `{ad}.text.txt` olarak yazıyor (ekran görüntüsü ve kutu geometrisi değişmeden; yakalanmış sayfalar için yalnızca metin, aynı Chrome sürümüyle). Erk aynı dikdörtgenleri Parley satırlarından hesaplıyor (`text_boxes`): paragraf her metin düğümünün metninin hangi bayt aralığına gittiğini tutuyor, her satırın kümeleri yerleştirildikleri yerle eşleniyor. Metin düğümleri iki tarafta aynı numaralanıyor (gövdedeki, yalnızca boşluk olmayan, `script`/`style`/`template` dışındaki düğümler, belge sırasıyla). Tolerans 1 px |
| — | **Ölçüm hatalarım:** daralan bir boşluğu, onu yazan metin düğümü yerine bir sonraki metnin başına sayıyordum; Chrome boşluğu yazıldığı düğümde sayar ("a " bir `<b>`'den önce, " dünya" bir `</b>`'den sonra). Farkların neredeyse hepsi tam bir boşluk genişliğiydi (16 px Noto Sans'ta 4,16 px). Ayrıca satır sonundaki boşluğu bir satır içi kutudan (inline-block) önce de atıyordum, ve gizli (`visibility: hidden`) metni ölçmüyordum; Chrome ikisini de ölçüyor |
| — | **Sonuç: 17 sayfadaki 163 metin satırının 159'u Chrome'la 1 px içinde.** inline-boxes, inline-styles, vertical-align ve text-transform tamamen tutuyor: bu sayfaların düşük piksel skoru yerleşim değil glif çizimi. Kalan dört satır gerçek hata ve testte gerekçeli `KNOWN_TEXT_DIFFERENCES` listesinde; liste iki yönlü: başka bir fark testi kırar, listelenen bir fark kaybolursa da (liste eskimesin) |
| — | **Bulunan iki gerçek hata:** (1) satır içi kutu parçalanması (`borders` sayfasında üç satır): kırılan kenarlıklı span 6 px'lik sol kenarlığını önceki satırda bırakıyor, metni yeni satırda o kadar solda başlıyor; M2.6'da. (2) Bir satır içi elemanın `position: relative`'i metnini kaydırmıyor (`settings` sayfasındaki rozetin `top: -1px`'i); M2.7'de |
| Ölçüm tabanı | `examples/perf/long-page.html` (20 bölüm, ~3000 CSS pikseli) ve `measure --frames`: sayfa renderer iş parçacığına bir kez gidiyor, her kare tuttuğu belgenin yeniden boyanması. **Ayrıştırma ihmal edilebilir:** tam kare ile ayrıştırmalı `render_html` aynı çıkıyor (aşağıda); M2'de her durum değişikliğinin bedeli stil, layout ve boyama. M1'in ölçümüne göre 800 × 600'de sabit maliyetin çoğu piksel sayısıyla orantılı |

| Sayfa (800 × 600) | `render_html`, 30 çağrının medyanı | Tutulan belgenin yeniden boyanması, medyan |
|---|---|---|
| `nodes-1000.html` | 71,16 ms | 72,11 ms |
| `long-page.html` | 36,78 ms | 31,20 ms |
| `settings.html` | 12,52 ms | 15,83 ms |

İki yol arasındaki farklar gürültü ve iş parçacığı ile mesaj yükü: ayrıştırma
kazancı ölçülemeyecek kadar küçük. Bu sayılar M5'in tabanı.

Mutasyonlar (8/8 yakalandı): kelime aralığı 2 px kayıyor; `line-height:
normal` 2 px büyüyor; bilinen bir fark listeden çıkıyor; eşleşen bir düğüm
bilinen fark diye listeleniyor; gizli metin ölçülmüyor; satır içi kutudan
önceki boşluk atılıyor; daralan boşluk onu yazan düğüme sayılmıyor; düğümün
kendi boşlukları ayrı aralıklara bölünüyor (ilk ikisi yerleşim hatası, gerisi
ölçümün ve listenin kendisi).

**M2.1 (2026-10-05).** Girdi hattı, hit-test, tıklama, denetim.

| Plan ne diyordu | Gerçek |
|---|---|
| `hit.rs` ve `events.rs` | Hit-test ve tıklama `page.rs`'te (`Page`): son karenin isabet bölgelerini tutan kalıcı belge zaten orada. Odak olayları M2.2'de gerekirse ayrılır |
| Kutular boyama sırasının tersinden gezilir | Display list'e boyanmayan ve dökümde görünmeyen `Hit { node, frame }` öğeleri giriyor: her elemanın kenarlık kutusu, kendi arka planıyla aynı yerde; her metin satırı için metnin elemanı. Böylece yığın bağlamları, `z-index` ve akış boyamayla aynı sıradan geliyor, ayrı bir sıralama yok; noktanın altındaki son öğe en üstteki. `pointer-events: none` ve `visibility: hidden` öğe üretmiyor |
| — | **Boş tuval kök elemanın:** testteki sayfada `html` ve `body`'nin yüksekliği 0 (bütün çocuklar mutlak konumlu), dolayısıyla hiçbir kutunun örtmediği noktada hedef çıkmıyordu. Chrome görüntü alanındaki böyle bir noktayı kök elemana verir (tuval onundur); Erk de öyle. Görüntü alanı dışı hedefsiz |
| Basma ve bırakma bir tıklama olur | Birincil tuşun basıldığı ve bırakıldığı düğümlerin en derin ortak atası tıklanır (iki eleman arasında kayan imleçte Chrome gibi); yol hedeften kök elemana elemanlar, metin düğümü değil. Diğer tuşlar ve karışık basma/bırakma tıklama üretmez |
| — | **Fare girdisi kare çizmez:** M2.1'de girdi henüz hiçbir stili değiştirmiyor; renderer döngüsü yalnızca belge, boyut, ölçek, font, kaynak ya da vurgu değişince çiziyor (`changed`). Hareket başına tam kare M2.2'de `:hover` ile geliyor |
| `Query` (M2.4) | Testlerin düğüm bulması için şimdiden: yalnızca `#id`. Tam seçici M2.4'te |
| Kabuk winit olaylarını mantıksal piksele çevirir | `CursorMoved` fiziksel konumu ölçek faktörüne bölüyor; sol, sağ, orta tuş birincil, ikincil, orta. Olaylar henüz kabukta kimseye gitmiyor (abonelik M2.4'ün demo host'unda). Kabuğun eşlemesinin otomatik testi yok: winit olayı üretmek pencere istiyor; renderer tarafı `tests/input.rs`'te mesaj protokolüyle sınanıyor |

Mutasyonlar (10/10 yakalandı): hit-test öndeki yerine arkadaki kutuyu
seçiyor; `pointer-events: none` yok sayılıyor; gizli metin hedef oluyor;
metin elemanı yerine bloğu hedefliyor; tıklama ortak ata yerine bırakılan
elemana gidiyor; her tuş tıklıyor (ilk hâlinde kaçtı, karışık tuş testi
eklendi); tuval kök elemana düşmüyor (betiğim mutasyonu yanlış uyguladı,
elle uygulanınca iki test kırıldı); vurgu sayfanın altına çiziliyor; fare
girdisi kare çiziyor; sorgu etiket adını da eşliyor.

**M2.2 (2026-10-05).** Eleman durumu, odak, klavye.

| Plan ne diyordu | Gerçek |
|---|---|
| Durum erk-style'ın yan tablosundaki `ElementState`'e yazılır | erk-style'ın yeni genel türü `Interaction { hover, active, focus }` (düğüm kimlikleri; Stylo türü dışarı sızmıyor) ve `StyleEngine::style_with`. Yan tablo doldurulurken `:hover` ve `:active` elemanın kendisine ve atalarına, `:focus` yalnızca elemana, `:focus-within` elemana ve atalarına yazılıyor. Belgede olmayan (eski nesilli) bir kimlik hiçbir elemanı duruma sokmuyor |
| Tam yeniden stil çalışır | Çalışıyor, ama her fare hareketinde değil: Stylo stil sayfalarının hangi durumlara bağlı olduğunu biliyor (`has_state_dependency`); `Styles::react_to` bunu söylüyor ve sayfa yalnızca seçicilerin kullandığı bir durum değişince yeniden çiziliyor. `:hover` kuralı olmayan sayfada fare hareketi kare çizmiyor; aynı eleman içinde hareket de |
| Tıklama odaklanabilir elemanı odaklar | Basma (bırakma değil, tarayıcılardaki gibi) basılan düğümün kendisi ya da en yakın odaklanabilir atasına odağı veriyor; odaklanabilir yoksa odak kalkıyor (Chrome). Olay sırası: blur, focus, sonra bırakmada click. Odaklanabilir: `href`'li `a`, etkin `button`/`input`/`select`/`textarea` (`type=hidden` hariç), `tabindex`'li her eleman; son karede gösterilmiş olmalı (`display`, `visibility`) |
| Tab ve Shift+Tab belge sırasıyla gezer | Önce pozitif `tabindex`'ler küçükten büyüğe, sonra geri kalanlar belge sırasıyla (HTML'in sıralı odak sırası); negatif `tabindex` yalnızca fareyle odaklanır. Uçlarda başa sarıyor (gömülü bir motorda odağın gideceği tarayıcı çubuğu yok). **Bilinen basitleştirme:** sıra dışındaki bir elemandan (negatif `tabindex`) Tab ilk elemana gidiyor; tarayıcı belgedeki konumundan devam eder. `tabindex` HTML'in tamsayı kuralıyla okunuyor (baştaki boşluk, işaret) |
| Enter ve Space bir tıklama üretir | Enter basılınca odaktaki bağlantıyı ya da düğmeyi, Space bırakılınca düğmeyi tıklıyor (tarayıcılardaki gibi; Space bağlantıda bir şey yapmaz). Klavye tıklaması 0, 0'da |
| Klavye mesajı `ToRenderer::Key { key, state, modifiers }` | `ToRenderer::Key(KeyInput)`; `Key` motorun ayırdığı tuşlar (Tab, Enter, Space, Escape, karakter, diğer). Kabuk winit'in mantıksal tuşunu eşliyor (birim testli; boşluğu karakter olarak bildiren platformlar da Space); sentetik basmalar (pencere odak kazanırken basılı olanlar) gönderilmiyor. **Host'a tuş olayları** (`ERK_EVENT_KEY_DOWN`, `KEY_UP`) henüz yok: olayın tuş bilgisi taşıması gerekiyor, metin girdisiyle M5'te |
| Referans sayfası: durumlu düğmeler | `states.html`: diğer referans sayfaları gibi div ve bağlantılardan düğmeler, form ve liste (form kontrolleri M5). Chrome'un durumsuz görüntüsüyle içerik %96,39; 14 kutunun 14'ü, 13 metin satırının 13'ü 1 px içinde (fark yalnızca glif kenar yumuşatması). Erk testi (`the_states_reference_page_follows_the_pointer_and_the_focus`) sayfayı mesajlarla her duruma sokuyor: durumdaki elemanın kuralın verdiği renge döndüğünü ve kutusu dışında hiçbir pikselin değişmediğini doğruluyor |
| — | **Odak halkası yok:** Chrome odaktaki elemana bir `outline` çizer (`:focus-visible`); Erk `outline` çizmiyor. `:focus-visible` ile M5'te |

Mutasyonlar (27'den 26'sı yakalandı): durum atalara ulaşmıyor; eski nesilli
kimlik yine işaretleniyor; `:focus-within` hiç eşleşmiyor; hover değişimi
hiç çizilmiyor; hover ve odak, kuralı olmayan sayfada da çiziliyor;
`pointerleave` hover'ı bırakmıyor; basma yalnızca hedefin kendisini
odaklıyor; blur olayı yok; gizli, gösterilmeyen, devre dışı, `type=hidden`
ve `href`'siz elemanlar odaklanabiliyor; negatif `tabindex` sırada; pozitif
`tabindex` önde değil (betikteki ilk biçimi derlenmedi, derlenen biçimiyle
yakalandı); pozitifler sıralanmıyor; sıra başa sarmıyor; Shift yok
sayılıyor; Space basılınca tıklıyor; Space bağlantıyı tıklıyor; her odaktaki
eleman tıklanıyor; `tabindex`'in baştaki boşluğu atlanmıyor; tuşlar sayfaya
ulaşmıyor; durum değişimi hiç çizilmiyor; kabukta boşluk karakteri Space
değil. **Kaçan:** kabuğun sentetik tuşları göndermemesi; winit'in `KeyEvent`'i
testte kurulamıyor (platforma özel gizli alanı var), kabuğun fare eşlemesi
gibi pencere isteyen kısım otomatik testsiz.

**M2.3 (2026-10-05).** Kırpma, kaydırma, imleç.

| Plan ne diyordu | Gerçek |
|---|---|
| Display list'e kırpma katmanı | Kırpma, Appendix E'nin evrelerine uymuyor: bir kırpan kutunun içeriği birkaç evreye (arka planlar, satır içi içerik, konumlandırılmış katmanlar) dağılıyor, ve mutlak konumlu bir kutu, kapsayıcı bloğu dışındaysa, ebeveyninin içinde olduğu kırpmadan kaçıyor. Yeni `scroll.rs` her kutunun *kapsamını* hesaplıyor: onu içeren en içteki kırpan kutu (mutlak konumlu için kapsayıcı bloğunu içerenler, sabit konumlu için hiçbiri, görüntü alanı da değil). Her display öğesi kapsamıyla etiketli üretiliyor; son geçiş ardışık öğeleri kapsamlarının kırpmalarıyla (`PushClip`/`PopClip`) sarıyor. Bir opaklık grubu başladığı kırpmaları bitene kadar tutuyor; içinden kaçan bir kutu (yarı saydam bir kutunun içinde, kapsayıcı bloğu kırpmanın dışında olan mutlak konumlu kutu) kırpılı kalıyor: bilinen uç durum |
| — | Kökün `overflow`'u, `visible` ise body'ninki görüntü alanınındır (CSS Overflow 3 §3.3): o eleman kırpmıyor, `hidden` belgeyi kullanıcıya kaydırtmıyor |
| Kaydırma konumu belgeyle saklanır | `Page` kaydırma konumunu düğüm başına tutuyor, her karede kabın gidebileceği kadarına kırpıyor (pencere büyüyünce belge geri geliyor). Kaydırma aralığı kabın içindeki kenarlık kutularından ve metin satırlarından (kutusundan taşsalar da), dolgu kutusunun köşesinden ölçülüyor; Taffy'nin `scrollable_overflow_rect`'i kullanılmadı: Erk'in kendi paragraf düzeni onu doldurmuyor ve konumlar düzenden sonra DOM'a göre yeniden yazılıyor. **Mutasyonla bulunan hata:** ilk hâli yalnızca kutuları sayıyordu; yalnızca metin tutan bir kap hiç kaymıyordu ve testi, tekerlek belgeyi kaydırdığı için yanlış sebeple geçiyordu |
| Tekerlek en içteki kabı kaydırır, sınırda dışarı taşar | `ToRenderer::Wheel`: imlecin altındaki elemanın yolundaki kullanıcı kaydırabilir kaplar (`auto`, `scroll`, belge) içten dışa; her biri alabildiğini alıyor, kalanı dıştakine. `hidden` ve `clip` tekerlekle kaymıyor. Bir şey kaymadıysa kare çizilmiyor. Kabuk satır başına 40 CSS pikseli, dokunmatik yüzeyin fiziksel pikselini ölçeğe bölerek gönderiyor (birim testli) |
| Hit-test kaydırmayı hesaba katar | İsabet bölgeleri display list'ten geldiği için kaydırmayla zaten kayıyor; şimdi çevrelerindeki kırpmalarla da kesiliyor: kırpılmış içerik tıklanmıyor |
| Basit kaydırma çubukları (kaplama olarak) | Kaplama başparmakları (6 px, yarı saydam), yalnızca imleç kabın (belgeninki için sayfanın) üstündeyken. Böylece referans görüntüleri (imleçsiz) etkilenmiyor ve yer ayrılmıyor; Chrome `--hide-scrollbars` ile de yer ayırmıyor. Kabın çubuğu kendi satır içi evresinde çiziliyor: konumlandırılmış torunlar üstüne biniyor; belgeninki her şeyin üstünde |
| `cursor`: `FromRenderer::Cursor(shape)` | `Cursor`, CSS anahtar kelimeleri ve gizli imleç için `None`; değişince, kareden sonra gönderiliyor (`:hover` imleci değiştirebilir). `auto` metnin üstünde metin imleci (isabet bölgesi metin satırı mı, onu biliyor), başka yerde ok. UA'ya HTML'in `:any-link { cursor: pointer }` kuralı eklendi (bağlantı renkleri değil). İmleç görüntüleri yüklenmiyor |
| Referans sayfası: kırpılan ve kaydırılmış kaplar | `overflow.html`: kaydırılan liste, kesilen metin kutusu, uzun kelime, kırpan konumlu kap, kırpmadan kaçan rozet, yatay kaydırılan satır. Chrome'la içerik %93,35; 22 kutunun 22'si, 19 metin satırının 19'u 1 px içinde. **İlk hâli bir fark buldu:** `white-space: nowrap`'lı kutu Erk'te iki satır (M2.6'nın kararı); sayfadan çıkarıldı. Erk testi listeyi ve yatay satırı kaydırıp yalnızca o kabın piksellerinin değiştiğini doğruluyor |
| — | **WPT: 17 test geçmeye başladı**, düşen yok: normal-flow 508 → 511, flexbox 569 → 581 (`flexbox-overflow-*`, `overflow-area-*`), position 48 → 50 (iki sticky testi: kırpma doğru kutuyu gösteriyor) |
| — | **Bulunan eksik: `<br>` satırı kırmıyor.** Hiçbir kod onu ele almıyor; M2.6'ya eklendi |
| — | **Kırpma köşeleri yuvarlamıyor:** `border-radius`'lu bir kırpan kutu dikdörtgen kırpıyor |

Mutasyonlar (20/20 yakalandı, üçü ancak test eklenince ya da düzeltilince):
sabit konumlu kutu kırpmalarını tutuyor; mutlak konumlu kutu hiç kaçmıyor;
kökün `overflow`'u görüntü alanına gitmiyor; kenarlık kutusuna kırpılıyor;
yatay ve dikey konum kırpılmıyor (yatay için pencere genişleme testi
eklendi); kabın kendi içeriği kaymıyor ve metin aralığa girmiyor (**ikisi
gerçek bir hatayı gösterdi**, yukarıda; test yalnızca metin tutan kabı
belgeyi kaydırmadan sınayacak biçimde yeniden yazıldı); anonim kutular
aralığa girmiyor (test kabı sonuna kadar kaydırıyor); `hidden` kullanıcıyla
kayıyor; iç içe kırpma dıştakinin kaydırmasıyla kaymıyor; isabet bölgeleri
kırpılmıyor; tekerlek zincirlenmiyor; çubuk değişimi çizilmiyor; kabın ya da
belgenin çubuğu hiç gösterilmiyor; metin üstünde `auto` ok; bağlantıda el
yok; imleç hiç gönderilmiyor; opaklık grubu kırpmalarını tutmuyor (yarı
saydam kutunun içinden kaçan kutunun saydam kaldığını sınayan test eklendi).

**M2.4 (2026-10-05).** İlk değişiklik ve sayaç demosu.

| Plan ne diyordu | Gerçek |
|---|---|
| `SetText { node, text }` ve `Query { request, selector }` → `QueryResult { request, node }` | `Query` `erk_query`'deki gibi bir `scope` da alıyor (`None`: belge). Yanıtlar `Result`: p1-contract §9'un durum kodları aynı numaralarla (`Status::InvalidArgument = 1`, `StaleNode = 2`). `SetText { request, node, text }` `FromRenderer::Done { request, result }` ile yanıtlanıyor; yalnızca başarılı bir değişiklik kare çiziyor |
| Eski bir `NodeId` hata olarak döner, çökmez | 0 ve hiçbir id'nin biçimi olmayan sayılar `InvalidArgument`, düğümü gitmiş id `StaleNode`. Denetim kaldırılınca renderer iş parçacığı eski id'de paniğe düşüyor (mutasyon): denetim tam olarak bunu önlüyor |
| — | **Sözleşmeye aykırılık bulundu ve düzeltildi:** her `Load` yeni bir arena kuruyordu; iki belgede de nesiller 1'den başladığı için önceki sayfanın bir id'si yeni sayfanın bir düğümünü gösterebiliyordu (p1-contract §2: "`erk_load_html` yeni bir arena kurmaz"). `Document::load_html` aynı arenaya ayrıştırıyor: eski düğümler önce siliniyor, nesilleri artıyor |
| `erk_node_set_text` | erk-dom'a ilk değişiklikler: `remove` (alt ağaç ve `<template>` içeriği birlikte; id'ler eskiyor, boşalan slotlar yeni nesille yeniden kullanılıyor), `set_text` (DOM'un `textContent`'i: metin ve yorum düğümünün verisi yerinde, elemanın çocukları tek bir metin düğümüyle, boş metinde hiçbiriyle değişiyor; belge düğümünde bir şey olmuyor) |
| `erk_query`, CSS seçici | `erk_style::query`: Stylo'nun seçici ayrıştırıcısı ve eşleştiricisi, belge sırasıyla, kapsamın kendisi hariç; `:scope` kapsam; `:hover` ve `:focus` gibi durumlar kullanıcının durumuna göre. **Bulunan sınır:** `:disabled` eşleşmiyor: form durumları erk-style'da yok (M5, css-support'ta öyle) |
| Sayaç demosu ve demo host (Rust, kabukta) | `examples/counter.html` ve `erk-shell/src/counter.rs`: host mesajlar üzerinde küçük bir durum makinesi; sayfa yüklenince `#count`, `#increment`, `#decrement`'i sorguluyor, yolu bir düğmeden geçen tıklamada sayacı değiştirip `#count`'un metnini ayarlıyor. Bu düğmeleri olmayan sayfalara dokunmuyor. Pencerede: `erk examples/counter.html`; düğmeler `<button>` olduğu için Tab, Enter ve Space de sayıyor |
| Otomatik test: tıklama gönderilir, kare altın görüntüyle doğrulanır | `clicks_count_and_the_page_shows_the_number`: üç kez artır, bir kez azalt; her değişikliğin yanıtı ve ardından karesi bekleniyor; imleç sayfadan çıkınca kare `tests/golden/counter-2.png` ile piksel piksel aynı (son tıklanan düğmenin odak halkası görüntünün parçası). Düğmelerin yeri sayfanın kendi düzeninden (`element_boxes`). **İlk hâli yarışlıydı** (tam test koşusunda bir kez 60 saniyede takıldı): sorguların yanıtı, ilk karenin çizildiği toplu işlem sırasında geliyor; test hemen tıklayınca tıklama aynı toplu işleme, ilk kareden önce düşüp boş isabet listesine çarpıyor ve kayboluyordu. Test artık ilk kareyi de bekliyor. Motorun davranışı yerinde: henüz çizilmemiş bir sayfada tıklanacak bir şey yok |

Mutasyonlar (16/16 yakalandı): `remove` çocukları ya da `<template>`
içeriğini bırakıyor; belge düğümü silinebiliyor; `set_text` çocukları
silmeden ayırıyor; boş metne boş bir metin düğümü ekleniyor; yükleme eski
düğümleri bırakıyor; her sayfaya yeni arena; değişiklik kare çizmiyor;
başarısız değişiklik kare çiziyor; eski id geçiyor (renderer paniğe düşüyor);
0 eski sayılıyor; bozuk seçici boş sonuç veriyor; ilk değil son eşleşme;
`:scope` yok; host yol yerine yalnızca hedefe bakıyor (düğmenin içindeki bir
simgeye tıklama sayılmazdı; test eklendi); azaltma yok.
