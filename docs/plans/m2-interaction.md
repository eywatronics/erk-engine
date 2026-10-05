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
crates/erk-renderer/src/document.rs     kalıcı belge: DOM, eleman durumu, kaydırma, son layout (M2.0)
crates/erk-renderer/src/hit.rs          hit-test: boyama sırasının tersinden (M2.1)
crates/erk-renderer/src/events.rs       tıklama ve odak olayları, yayılma yolu (M2.1, M2.2)
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

- [ ] Mesajlar (düz veri): `ToRenderer::Pointer { kind: Move | Down | Up |
  Leave, x, y, button, modifiers }` (mantıksal piksel). Kabuk winit'in fare
  olaylarını ölçekten bağımsız mantıksal piksele çevirip gönderir.
- [ ] Hit-test: son layout'un kutuları boyama sırasının tersinden gezilir
  (yığın bağlamları, `z-index`, akış); metnin üstündeki nokta metnin
  elemanını verir; `pointer-events: none` atlanır; `visibility: hidden` hedef
  olmaz.
- [ ] Tıklama: aynı eleman (ya da ortak ata) üstünde basma ve bırakma bir
  `ERK_EVENT_CLICK` olur; `FromRenderer::Event { kind, target, path, x, y,
  modifiers }`. `path` hedeften köke düğümler: capture, target ve bubble
  sırasını host bu yoldan kurar. Otomatik test: bir tıklama doğru `NodeId`'yi
  bildiriyor (kabulün maddesi).
- [ ] Denetim (p1-contract §8.1): `ToRenderer::InspectAt { x, y }` hit-test'in
  `NodeId`'sini döndürür; `ToRenderer::Highlight { node }` seçili düğümün
  kutularını bir kaplamayla çizer. **Muhafız** (p1-contract §11): vurgu
  açıkken ve kapalıyken DOM dökümü ve hesaplanmış stiller aynı; display
  list'te yalnızca kaplama öğesi farklı.

### M2.2: Eleman durumu ve odak

- [ ] `:hover`: imlecin altındaki eleman ve ataları; `:active`: basılı tuşun
  elemanı ve ataları; `:focus`, `:focus-within`. Durum erk-style'ın yan
  tablosundaki `ElementState`'e yazılır (bugün yalnızca bağlantılar için
  dolu) ve tam yeniden stil çalışır.
- [ ] Odak: tıklama odaklanabilir elemanı (`button`, `a[href]`, `tabindex`)
  odaklar; Tab ve Shift+Tab belge sırasıyla gezer; odaktaki düğmede Enter ve
  Space bir tıklama üretir. `ERK_EVENT_FOCUS` ve `ERK_EVENT_BLUR`. Klavye
  mesajı: `ToRenderer::Key { key, state, modifiers }`.
- [ ] Referans sayfası: `:hover` ve `:focus` stilli düğmeler; Chrome
  görüntüsü durumsuz, Erk testi durumu mesajla verip pikselleri doğrular.

### M2.3: Kaydırma, kırpma, imleç

- [ ] `overflow: hidden | auto | scroll` çocukları kırpar (display list'e
  kırpma katmanı); bu M1'de açık kalan "çocuklar kırpılmıyor" sınırını da
  kapatır.
- [ ] Kaydırma kapları ve kök görüntü alanı: `ToRenderer::Wheel { dx, dy,
  x, y }` imlecin altındaki en içteki kaydırılabilir kabı kaydırır, sınırda
  dışarı taşar; kaydırma konumu belgeyle saklanır. Hit-test kaydırmayı
  hesaba katar. Basit kaydırma çubukları (kaplama olarak).
- [ ] `cursor` özelliği: `FromRenderer::Cursor(shape)`, kabuk winit'e verir.
- [ ] Otomatik test: uzun bir sayfa tekerlekle kayıyor ve görünen içerik
  değişiyor (kabulün maddesi). Referans sayfası: kırpılan ve kaydırılmış kaplar.

### M2.4: İlk değişiklik ve sayaç demosu

- [ ] M4'ün `Mutation` API'sinin ilk parçası: `ToRenderer::SetText { node,
  text }` (`erk_node_set_text`) ve `ToRenderer::Query { request, selector }`
  → `FromRenderer::QueryResult { request, node }` (`erk_query`, CSS seçici).
  Eski bir `NodeId` hata olarak döner, çökmez (test).
- [ ] Sayaç demosu: `examples/counter.html` ve demo host (Rust, kabukta):
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
