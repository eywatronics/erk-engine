# Erk Engine — P2: Artımlı render mimarisi (M5)

- **Tarih:** 2026-10-01
- **Durum:** Tasarım. Kod yok; M5 başlayınca bu belge o taşın planına
  dönüşür, açık sorular (§9) M5'in ilk adımında kapanır.
- **Önceki belgeler:** [p1-embedded.md](p1-embedded.md),
  [p1-contract.md](p1-contract.md), [roadmap.md](../plans/roadmap.md) M2 ve
  M5.

Bu belge dışarıdan gelen bir M5 mimari önerisinin, **Erk Invalidation Core
(EIC)**, üç sürümünün, onlara yapılan eleştirilerin ve uygulama tuzakları
notunun değerlendirmesidir. **Bu, nihai hâlidir:** v3 ile birlikte tasarım
tartışması kapandı. Bundan sonraki değişiklikler M5'in yürütme notlarına
ölçüm ve kodla gelir.
Önerinin adı korunur. İçeriği, Erk'in bugünkü kodu ve kullandığı
kütüphanelerin zaten yaptığı işler üzerine yeniden kurulur.

---

## 1. Problem

M2 bilerek kaba: her durum değişikliği (hover, kaydırma, bir harf) tam
yeniden stil, layout ve boyama ister. M5'in işi bunu değişenle orantılı hale
getirmek. Kabul ölçütü ölçüm: 10 bin düğümlü bir belgede bir metin alanına
yazarken p95 kare süresi, M2'nin tam yeniden hesabına göre.

Bugün her kare sıfırdan kuruluyor. `render_html` belgeyi ayrıştırıyor,
Stylo tüm ağacı stilliyor, `layout::build` yan tabloyu ve Taffy önbelleğini
her seferinde yeniden kuruyor, display list baştan üretiliyor. M5'te kalıcı
olacaklar şunlar: DOM (M4'ten beri zaten kalıcı), stil verisi, layout yan
tablosu ve önbelleği, şekillendirilmiş metin, display list.

## 2. Önerinin değerlendirmesi

| Öneri | Karar |
|---|---|
| Tek bir invalidation sözlüğü: stil, metin, layout, boyama, erişilebilirlik için bit kümesi | **Alındı** (§3.3). Yön bilgisi (kendisi, alt ağaç, ebeveyn) bitlerde |
| `InvalidationCause`: neden kirli | **Alındı.** Hata ayıklama, profil ve M7'nin DevTools'u için ("Düğüm 742 neden yeniden yerleşti? Çocuk 743'ün içsel boyutu değişti"). Eleştirinin en değerli eklemesi |
| Mutation Journal, birleştirme (coalescing), iç içe transaction | **Alındı, M4'ün üstüne** (§3.2). `Mutation` toplu API'si zaten sözleşmede; journal onun kare içindeki birikimi, transaction da bağlamaların kullandığı kapsam |
| Stil invalidation'ını kendimiz yazmak (`STYLE_SIBLING`, `:has()` için kurallar) | **Reddedildi.** Stylo bunu Firefox için zaten yapıyor: eleman anlık görüntüleri (snapshot), yeniden stil ipuçları, kardeş ve `:has()` invalidation'ı, hesaplanan stil farkından boyama/layout hasarı. Erk'in işi bunu bağlamak ve hasarı kendi bitlerine çevirmek (§3.4). İkinci bir seçici invalidation motoru, Stylo'yla çelişen bir kopya olurdu |
| CSS özelliğinden doğrudan `PropagationRule` tablosu (v1) | **Düzeltildi.** Tablonun satırları yanlıştı. `contain: layout` tek başına boyutu içerikten ayırmaz, bunu `contain: size` yapar. `visibility: hidden` alt ağacın boyamasını emmez: görünür torunlar yine boyanır, Erk bunu bugün uyguluyor ve test ediyor. Sabit `width`/`height` min/max-content katkısını kesmez. v2'nin bağımlılık sınıflandırması doğru yön; Erk'te statik sınırlar ve dinamik erken kesme olarak (§3.5) |
| Containment'ı birinci sınıf yapmak | **Alındı, iki yoldan** (§3.5): hesaplanan stilden çıkan **yeniden yerleşim sınırları** (Flutter'ın relayout boundary'si; CSS'te `contain: size layout`) ve **erken kesme** (yeniden hesaplanan çıktı eskisiyle aynıysa yayılma durur; Salsa'nın early cutoff'u). İkincisi CSS yazılmasını beklemez, her sabit boyutlu kutuda kendiliğinden çalışır |
| Tek global epoch (v1), aşama başına epoch (v2) | **Sadeleştirildi.** Kirlenme bitleri ve Taffy'nin girdi anahtarlı önbelleği aynı soruyu zaten yanıtlıyor. Epoch yalnızca kare sayacı olarak kalır; imza (hash) yalnızca içeriğin gerçekten değişip değişmediği şüpheli yerlerde, metin şekillendirme önbelleğinde kullanılır |
| Damage region: birleşim değil küme, eşik aşılınca tam kare | **Alındı, arka uçtan bağımsız** (§3.7). Erk neyin yeniden çizileceğini söyler, arka uç bunu nasıl kullanacağına karar verir. Döşeme boyutu sabit değil parametre; 16, 32 ve 64 ölçülerek seçilir |
| `RenderBackend` trait'i | **Alındı.** Bugün `vello_cpu`; M2'de `vello_hybrid`. İkisi de aynı damage bilgisini alır |
| Work Graph ve Rayon ile düğüm/aşama düzeyinde paralel yürütme | **M5'te reddedildi, M9'da ölçüm kapısı** (§3.9). Paralellik zaten doğru yerlerde var: Stylo'nun paralel geçişi, renderer iş parçacığı, `vello_cpu`'nun çok iş parçacıklı rasterı. Taffy tek iş parçacıklı. Ölçülmemiş bir zamanlayıcı tek kişilik bir projede en pahalı tür karmaşıklık. `DependencyKind` sınıflandırması M9'a not olarak kalır |
| Signal katmanı motorda | **Reddedildi, bağlamalara taşındı** (§3.10). Durum host'undur (p1-embedded §2). Sinyaller Python, Go ve JS bağlamalarında yaşar; bir sinyal değişimi `Mutation` üretir, aynı journal'a girer. v2'nin "hiçbir şey çekirdeği atlamaz" ilkesi böylece yapı gereği sağlanır |
| Değişmez snapshot, `Arc` ile yapısal paylaşım, worker'a takas | **Reddedildi.** Sözleşme (p1-contract §1.1) belgeyi UI iş parçacığında, rasterı kendi iş parçacığında tutuyor. İkisi arasında düz veri bir display list gidiyor. DOM'un paylaşılan bir kopyası gerekmiyor; paylaşım, düz veri mesaj kuralını (proje kuralları) bozardı |
| AccessKit "kapalıyken sıfır maliyet" | **Düzeltilerek alındı.** AccessKit bağdaştırıcıları ağacı yardımcı teknoloji etkinleşince ister. Erişilebilirlik ağacının kurulması ve eşitlenmesi bu duruma göre tembeldir. DOM'daki öznitelikler (`role`, `aria-*`) her zaman vardır |
| `:has()`'ı M5'in sonunda bir UA özelliği olarak almak | **Öne alındı, doğrulama vakası olarak.** `.card:has(input:checked)` atadan torune bağımlılığın uçtan uca çalıştığını gösterir: Stylo'nun ipucu → `.card` stili → boyama hasarı. Kalıcı stilin kabul testlerinden biri |
| Invalidation cebiri, biçimsel özellikler | **Alındı, test olarak** (§3.11). Özellikler (boş girdi boş çıktı, tekrar uygulama aynı sonuç, monotonluk) özellik tabanlı testlerle doğrulanır. v1'deki `clamp` taşıma boşken de bit eklediği için yayılma hiç durmuyordu; tanım bunu düzeltir |
| Benchmark kabul ölçütleri (≤ 8 düğüm, < 200 µs, ≥ 50x) | **Ölçüm olarak alındı, hedef olarak değil** (§4). Proje kuralı: sayılar ölçümden konur. Taban M2'nin tam yeniden hesabı; "EIC kapalı" karşılaştırması bu yüzden bedava |
| "O(1) yayılma, containment sayesinde" | **Düzeltildi:** yayılma, düğümden en yakın sınıra olan mesafe kadar sürer, O(h) |
| "Style/layout/paint/a11y/signal'i birleştiren ilk modern Rust UI motoru" | **Alınmadı.** Masonry/Xilem'in geçiş bayrakları, Flutter'ın sınırları, Blink'in LayoutNG önbelleği bu alanın kanıtlanmış işleri. Erk'in iddiası §6'da, "ilk" ve "en hızlı" demeden |
| M5.0–M5.13, on dört adım | **Sıkıştırıldı:** dokuz adım (§5). İlk adım ölçüm altyapısı; formlar, IME ve davranışı olan elemanlar M5'te kalıyor |

v3'ün ve uygulama tuzakları notunun değerlendirmesi:

| Öneri | Karar |
|---|---|
| Work Graph bir aşama zamanlayıcısı değil, veri bağımlılığı grafiği; `DependencyKind` kenarları (stil girdisi, ebeveyn/kardeş/ata stili, çocuk layout'u, ebeveyn kısıtı, kaynak) | **Sözlük olarak alındı, zamanlayıcı yine M9'da** (§3.9). Kenar türleri iki işe yarar: nedensellik zincirinin dili (§3.3) ve M9'daki değerlendirmenin başlangıcı. Stil kenarları (`SiblingStyle`, `AncestorStyle`) Stylo'nun içinde kalır; `:has()` Erk'te ayrı bir kenar türü olarak değil, Stylo'nun ipucu olarak gelir |
| `RenderDependencyMap`: DOM düğümü → render parçaları → hasar | **Alındı** (§3.7). Örnek vaka belgeye girdi: `color` değişince yalnızca o düğümün arka plan parçası, `width` değişince düğümün ve alt ağacının bütün parçaları |
| Nedenler düğümde değil journal'da | **Alındı;** zaten halka tampondaydı (§3.3). Açma kapama derleme profiline değil bir Cargo özelliğine (`inspect`) bağlı: geliştirme derlemelerinde açık, yayında kapalı. Yalnızca `debug_assertions`'a bağlamak, M7'nin DevTools'unu yayın derlemesinde imkânsız kılardı |
| Biçimsel cebir: tekrar uygulama, monotonluk, yakınsama, birleşim üzerine dağılma | **Alındı, `clamp` olmadan** (§3.11). v3'ün tanımı `clamp`'i sonuca uyguluyor; boş girdide bile bit eklediği için `P(∅) = ∅` bozuluyor ve yürüyüş durmuyor. Sınırın kendi `LAYOUT_SELF`'i yayılmanın değil tohumun işi. Birleşim üzerine dağılma yayılmanın birleştirmeyle uyumunu kanıtlar; journal birleştirmesinin doğruluğunu (son DOM durumu sırayla uygulamayla aynı) ayrıca fuzz kanıtlar |
| Beş epoch (kare + dört aşama) | **Alınmadı:** kare sayacı yeter (§2'deki karar). Taffy'nin önbelleği girdi anahtarlı; bitler ve erken kesme aşama epoch'larının yanıtladığı soruyu zaten yanıtlıyor. Ölçüm aksini gösterirse M5'in yürütme notlarına girer |
| Döşeme boyutu parametre, uyarlanır döşeme sonra | **Alındı:** M5'te sabit varsayılan ve ayar; 16, 32, 64 ölçülüp kazanan varsayılan olur. Uyarlanır döşeme M9'da (kompozitör), önerinin dediği M6'da değil: M6 bağlamaların taşı |
| Tuzak 1: `HashMap<NodeId, …>` yerine yan tablolar | **Alındı, kural olarak** (§3.12). Düğüme bağlı her veri `NodeId::index()` ile indekslenen düz bir vektörde, nesil denetimiyle; layout ve stil durumunun bugün durduğu gibi. Taslaktaki journal haritası da yan tabloya çevrildi (§3.2) |
| Tuzak 2: Work Graph'ta döngü ve kilitlenme | **M9'a yazıldı.** Planlanan bağımlılık türleriyle graf yönlü ve döngüsüz (DAG) kurulmalıdır. Bu bir varsayım değil, kabul testidir: topolojik sıralama ve döngü tespiti kapının kabulüne girer; `Resource`, kardeş ve alt ağaçlar arası kenarlar eklendikçe test de genişler. Yüzdelik çocuklu içsel boyut gibi CSS döngüleri grafla değil, CSS'in döngüsel yüzde kurallarıyla Taffy'de çözülür |
| Tuzak 3: EIC `erk-dom`'a giremez, `erk-dom` kirlenme bitlerini bilmez | **Alındı** (§3.12). Yeni `erk-invalidation` crate'i yalnızca `erk-dom`'a bağımlı; muhafızı kendisiyle aynı PR'da |

## 3. Mimari

### 3.1 Akış

```
host / bağlama (Rust, Python, Go, erk-script)
        │  Mutation'lar (sinyaller burada Mutation'a çevrilir)
        ▼
Transaction ──► MutationJournal (kare içinde birikir, birleşir)
                     │  kare sınırı
                     ▼
      DOM'a uygula + tohum invalidation (bit + neden)
                     │
       ┌─────────────┼──────────────────────┐
       ▼             ▼                      ▼
  Stylo: snapshot   layout: kirlenme     erişilebilirlik:
  → yeniden stil    yukarı, sınırda      etkinse eşitle
  → stil hasarı     dur, erken kes
       │             │
       └──────┬──────┘
              ▼
   display list parçaları (yalnızca kirli kutular)
              │  hasar bölgesi
              ▼
   renderer iş parçacığı: RenderBackend (vello_cpu / vello_hybrid)
```

Akış tek iş parçacığında, UI iş parçacığında yürür; yalnızca raster ayrı
iş parçacığındadır (p1-contract §1.1). Stylo kendi içinde paralel gezebilir.

### 3.2 Mutation journal ve transaction

M4'ün `Mutation` toplu API'si (oluştur, ekle, sil, metin, öznitelik, sınıf,
satır içi stil) değişmez. M5 kare içi birikimi ekler:

```rust
pub struct MutationJournal {
    records: Vec<Mutation>,
    /// Per node (a side table indexed by `NodeId::index()`): its last record
    /// for each slot this frame, for coalescing. Only the touched entries
    /// are reset at the frame boundary.
    slots: Vec<SlotRecords>,
    touched: Vec<NodeId>,
}
```

Birleştirme kuralları: aynı düğümün metni ya da aynı özniteliği için son
değer kazanır. Aynı karede eklenip çıkarılan sınıf iptal olur. Oluşturulup
aynı karede silinen düğüm hiç uygulanmaz. Kare sınırında journal tek seferde
uygulanır; "merhaba" yazarken beş değil bir metin değişikliği.

Transaction bağlamaların kapsamıdır (`with app.transaction(): ...`). İç içe
olabilir, yalnızca en dıştaki kapandığında journal kare kuyruğuna girer.
C-ABI'de bu `erk_batch` çağrısının kendisidir; iç içelik bağlamada sayılır.

### 3.3 Invalidation: ne ve neden

```rust
bitflags! {
    pub struct Invalidation: u16 {
        const STYLE_SELF      = 1 << 0;  // seeded from Stylo's restyle hint
        const STYLE_SUBTREE   = 1 << 1;
        const TEXT_SHAPE      = 1 << 2;  // the paragraph must be shaped again
        const LAYOUT_SELF     = 1 << 3;
        const LAYOUT_ANCESTOR = 1 << 4;  // the size may leak to the parent
        const PAINT_SELF      = 1 << 5;
        const PAINT_SUBTREE   = 1 << 6;
        const A11Y_SELF       = 1 << 7;
        const A11Y_SUBTREE    = 1 << 8;
        const HIT_TEST        = 1 << 9;
    }
}

pub enum InvalidationCause {
    Text, Attribute, Class, InlineStyle, State /* hover, focus, ... */,
    ChildLayout(NodeId), ParentLayout, Resource, Viewport,
}
```

Bitler düğüm başına bir yan tabloda tutulur, `NodeId::index()` ile, layout ve
stil durumunun durduğu gibi. Neden düğümde tutulmaz. Her tohum ve her
yayılma adımı `(sıra, düğüm, bitler, neden)` olarak bir halka tampona
yazılır; böylece zincir geriye doğru okunabilir:

```
Düğüm 742: STYLE + LAYOUT
  ← ChildLayout(743)
    ← Text (düğüm 743)
      ← Mutation, sıra 1284
```

Kayıt `inspect` Cargo özelliğinin arkasındadır: geliştirme derlemelerinde
ve DevTools'lu derlemelerde (M7) açık, yayında kapalı ve sıfır maliyetli.
Sıcak yolda (120 Hz sürükleme) açık olması ölçülür.

### 3.4 Stil: Stylo'nun invalidation'ı

Erk seçici invalidation'ı yazmaz. Değişiklik uygulanmadan önce etkilenen
elemanın anlık görüntüsü (öznitelikler, sınıflar, durum) alınır. Stylo
snapshot'ı yeni hâlle karşılaştırıp hangi elemanların yeniden stilleneceğini
çıkarır. Torunlar, kardeşler ve `:has()` ile atalar buna dahildir. Yeniden
stillenen her elemanın eski ve yeni hesaplanmış stili arasındaki fark bir
hasar üretir. Erk bu hasarı bitlere çevirir: yalnızca renk değiştiyse
`PAINT_SELF`; yazı tipi değiştiyse `TEXT_SHAPE | LAYOUT_SELF`; kutu
özellikleri değiştiyse `LAYOUT_SELF`; `display` değiştiyse ağaç yeniden
kurulur.

Bu, `erk-style`'ın `TElement` uygulamasına snapshot desteği ekler. Sabit beş
`unsafe fn` imzalı yüzeye dokunup dokunmadığı açık sorudur (§9).

### 3.5 Layout: sınırlar ve erken kesme

Taffy'nin modeli zaten artımlı: her düğümün önbelleği girdi anahtarlıdır
(bilinen boyutlar, kullanılabilir alan). Kirlenen bir düğümün önbelleği ve
atalarınınki temizlenir. Erk bunun üstüne iki şey ekler:

1. **Yeniden yerleşim sınırı.** Boyutu içeriğinden bağımsız olan bir kutuda
   yayılma durur: `contain: size layout`, ya da `width` ve `height` sabit
   uzunluk, yüzde değil, ve kutu flex/grid kalemi değil. Hesaplanmış stilden
   çıkarılır ve muhafazakârdır: emin olunamayan her durum sınır değildir.
   Flutter'ın relayout boundary'sinin CSS karşılığı.
2. **Erken kesme.** Kirli düğüm son girdileriyle yeniden yerleştirilir.
   Çıktısı (boyut, taban çizgileri) eskisiyle aynıysa atalarına
   `LAYOUT_ANCESTOR` gitmez, yalnızca alt ağaç yeniden yerleşir. Bir
   düğmenin metni aynı genişlikte kalırsa sayfanın geri kalanı yerinden
   oynamaz, ve bunun için kimsenin `contain` yazması gerekmez.

Paragraflar (M1.3) ve atomları için önbellek anahtarı satır kırma
genişliğidir. Atomun boyutu değişmediyse paragraf yeniden kırılmaz.

### 3.6 Metin

Paragraf, şekillendirilmiş ve kırılmış satırlarıyla kalıcıdır. Anahtar,
metin, stil aralıkları, atom boyutları ve kırma genişliğinden bir imzadır.
`TEXT_SHAPE` yalnızca bunlardan biri değişince şekillendirmeyi yeniler.
Hizalama ve satır kayması (M1.3) şekillendirme olmadan yeniden hesaplanır.

### 3.7 Boyama ve hasar

Display list kutu başına parçalara bölünür: arka plan, satır içi arka
planlar, glif run'ları, atom. Bir DOM düğümü birden çok parça üretebilir
(bir düğmenin arka planı, metni, odak halkası). Düğümden parçalarına
eşleme bir yan tablodur (`Vec<SmallVec<[ChunkId; 2]>>`, `NodeId::index()`
ile). Hasar, değişen her parçanın eski ve yeni sınırlarının birleşimidir.
Boyama sırası (CSS 2 Ek E) parçaların sırasıyla korunur.

Örnek vaka, EIC'nin ne kazandırdığını gösteren:

```
color: kırmızı → mavi            width: 100px → 120px
  Stylo hasarı: yalnızca boyama    Stylo hasarı: layout
  → PAINT_SELF (düğüm 42)          → LAYOUT_SELF (düğüm 42)
  → parça 17 yeniden üretilir      → düğüm ve alt ağacı yeniden yerleşir;
  → hasar: parça 17'nin sınırı       genişliği değişmezse (erken kesme)
                                     atalar yerinde kalır
                                   → 42'nin bütün parçaları ve yer değişen
                                     torunlarınınkiler; hasar: eski ve yeni
                                     sınırların birleşimi
```

```rust
pub struct DamageRegion {
    rects: SmallVec<[Rect; 8]>,
    /// Promote to a full frame above this share of the viewport (measured).
    full_above: f32,
}

pub trait RenderBackend {
    fn render(&mut self, list: &DisplayList, damage: &DamageRegion) -> Frame;
}
```

Hasarın nasıl kullanılacağı arka ucundur. `vello_cpu` yalnızca hasarlı
dikdörtgenleri kırpıp yeniden rasterlar. Kabuk `softbuffer`'ın hasarlı
sunumuyla yalnızca o bölgeyi ekrana gönderir. `vello_hybrid` (M2) makas
dikdörtgeniyle çalışır. Döşeme boyutu ve tam kareye terfi eşiği ölçümle
seçilen parametrelerdir.

### 3.8 Erişilebilirlik

`A11Y_*` bitleri AccessKit ağacının güncellemesini besler. Bağdaştırıcı
etkin değilken ağaç kurulmaz, bitler yalnızca birikir. Etkinleşince ilk ağaç
bir kez kurulur, sonra yalnızca kirli düğümler gönderilir. Rol, ad ve durum
özniteliklerden türetilir (p1-contract §6.1).

### 3.9 Paralellik

M5'te genel bir iş grafiği yok. Paralellik üç yerden gelir: Stylo'nun
paralel stil geçişi, renderer iş parçacığı ve `vello_cpu`'nun çok iş
parçacıklı rasterı. M9'da (kompozitör) ölçüm gösterirse, bağımsız alt
ağaçların layout'u ya da erişilebilirlik eşitlemesi için bir iş grafiği
değerlendirilir. O değerlendirmenin girdileri bugünden yazılı:

- **Kenarlar veri bağımlılığıdır, aşama sırası değil:** `LayoutInput`,
  `PaintInput`, `A11yInput`, `ChildLayout`, `SiblingLayout`,
  `ParentConstraint`, `Resource`. Stil kenarları Stylo'nun içinde kalır.
- **İş birimi alt ağaçtır, düğüm değil:** 10 bin düğüm için 10 bin iş,
  iş çalma yükünü patlatır.
- **Graf DAG olarak kurulmalı, bu da test edilmeli:** planlanan bağımlılık
  türleriyle graf yönlü ve döngüsüz kurulur. "Yapı gereği döngü olmaz"
  denmez: `Resource`, kardeş ya da alt ağaçlar arası kenarlar eklendikçe bu
  iddia her seferinde yeniden doğru olmak zorunda kalır. Topolojik sıralama
  ve döngü tespiti kapının kabul testidir; tespit edilen bir döngü,
  kilitlenme değil hata olur.

### 3.10 Sinyaller ve bağlamalar

Motorda sinyal yok. Bağlamalar (M6) sinyali kendi dillerinde sunar. Değer
değişince bağlama bir `Mutation` üretir (`SetText(#sayac, "5")`) ve aynı
journal'a koyar. Böylece React'in reconciler'ı ya da Solid'in sinyal grafiği
gibi ikinci bir mekanizma olmaz. Çekirdekte bir yol var, onu da her dil
kullanır. Sıcak yollar için (`bind_hot`) bir kaçış kapısı ancak M6'dan sonra,
ölçümle.

### 3.11 Invalidation cebiri

Bir kenardaki yayılma, çocuğun kuralı `R = (absorb, promote)` ile:

```
P(I, R) = ∅                              I = ∅ ise
P(I, R) = (I ∖ R.absorb) ∪ R.promote     değilse
```

`I` yalnızca bitlerdir; yön bitlerin içindedir (`*_SELF`, `*_SUBTREE`,
`LAYOUT_ANCESTOR`), neden ise hesaba girmeyen bir ek veridir. `clamp` yoktur:
bir sınırın kendi `LAYOUT_SELF`'i, yayılmanın değil sınır düğümün tohumunun
işidir.

Doğrulanacak özellikler (özellik tabanlı testlerle):

- **Boş girdi:** `P(∅, R) = ∅`. Kirlenme olmadan yayılma olmaz. v1'deki
  koşulsuz `clamp` ve v3'ün sonuca uygulanan `clamp`'i bunu bozuyordu.
- **Tekrar uygulama:** `P(P(I, R), R) = P(I, R)`, çünkü
  `((I ∖ A) ∪ B) ∖ A ∪ B = (I ∖ A) ∪ B`.
- **Monotonluk:** `I₁ ⊆ I₂ ⇒ P(I₁, R) ⊆ P(I₂, R)`.
- **Birleşim üzerine dağılma:** `P(I₁ ∪ I₂, R) = P(I₁, R) ∪ P(I₂, R)`.
  Bir karedeki iki değişikliği ayrı ayrı yaymak, birlikte yaymakla aynı
  bitleri verir; yayılma birleştirmeyle uyumludur. Journal birleştirmesinin
  kendi doğruluğu (birleşmiş journal'ın son DOM durumu, kayıtların sırayla
  uygulanmasıyla aynı) bir cebir özelliği değildir, fuzz ile doğrulanır.
- **Yakınsama:** ata zaten bu bitleri taşıyorsa yürüyüş durur. Bir
  değişikliğin yayılması en fazla en yakın emici sınıra olan mesafe `d`
  kadar sürer, O(d).

### 3.12 Kod yerleşimi

- **`erk-dom` kirlenmeyi bilmez.** En alttaki katman olarak kalır, hiçbir
  `erk-*` crate'ine bağımlı değildir.
- **Yeni crate `erk-invalidation`:** bitler, nedenler, yayılma kuralları ve
  cebir, yan tablolar, `MutationJournal` ve birleştirme. Yalnızca
  `erk-dom`'a bağımlıdır (`NodeId`, `Mutation`) ve tek başına test edilir.
  Muhafızı (`cargo tree`) kendisiyle aynı PR'da gelir.
- **Tüketiciler:** `erk-style` (stil hasarı → bitler), layout ve display
  list. M3'ten sonra bunları kare boyunca yöneten `erk` crate'i journal'ı
  kare sınırında uygular.
- **Yan tablo kuralı:** düğüme bağlı her veri `NodeId::index()` ile
  indekslenen düz bir vektörde, nesil denetimiyle durur. Düğüm anahtarlı
  `HashMap` yoktur: her değişiklikte hash, 10 bin düğümde önbelleği bozar.
  Yalnızca seyrek ve düğümle ilgisiz veri (örneğin kaynak kimlikleri)
  haritada durabilir.

## 4. Ölçüm planı

Taban M2'nin tam yeniden hesabı; her ölçüm onunla karşılaştırılır. Sayılar
ölçmeden yazılmaz, hedefler M5.0'daki ilk ölçümden konur.

| # | Senaryo | Ölçülen |
|---|---|---|
| B1 | 10 bin düğüm, bir metin değişikliği | Yeniden stillenen, yerleşen ve boyanan düğüm sayısı |
| B2 | Bir karede 100 metin değişikliği | Layout geçişi sayısı (birleştirme) |
| B3 | 1000 derinlik, en alttaki yaprağa sınıf, sınır 10 düzey yukarıda | Yayılma süresi; sınıra mesafeyle ölçeklenme |
| B4 | `.card:has(input:checked)` aç/kapa | Yeniden stil süresi ve stillenen eleman sayısı |
| B5 | `contain: size layout` içindeki değişiklik (yalnızca `contain: layout` değil: o, boyutu içerikten ayırmaz) | Sınırın dışındaki layout işi (beklenen: yok) |
| B6 | 5 bin düğümlü sayfada imleç yanıp sönmesi | Hasar alanı |
| B7 | 120 Hz'de sürükleme | p99 kare süresi |
| B8 | Erişilebilirlik kapalı ve açık, 10 bin düğüm | Erişilebilirlik ağacına harcanan süre |
| B9 | B1, artımlı ve tam yeniden hesap | Oran |
| B10 | Art arda 100 transaction | Her aşamadaki sayı ayrı ayrı: journal kayıtları, birleştirme sonrası değişiklikler, DOM'a uygulamalar, layout geçişleri, display list yeniden kurulumları, sunulan kareler. Tek başına "bir kare", birleştirmenin işini gösterip göstermediğini söylemez |
| B11 | Döşeme boyutu 16, 32, 64; B6 ve B7 senaryolarında | Hasar alanı ve kare süresi; kazanan varsayılan olur |

Ölçümler `measure` örneğinin yanına bir kıyas örneği olarak gelir.
Adlandırılmış bir makinede koşar ve plana yazılır, M1.0'daki ölçümler gibi.

## 5. M5 adımları

| Adım | İçerik | Kabul |
|---|---|---|
| M5.0 | Ölçüm altyapısı: B1–B11 senaryoları, M2'nin tam yeniden hesabı taban | Taban sayıları plana yazılmış |
| M5.1 | Mutation journal, birleştirme, transaction | 100 metin değişikliği bir uygulama; birleştirme kuralları testli |
| M5.2 | `erk-invalidation` crate'i: bitler, neden tamponu, cebir testleri, yan tablolar; crate'in bağımlılık muhafızı | Özellik testleri yeşil; bir mutasyon (koşulsuz terfi) yakalanıyor; muhafız kasıtlı bir bağımlılığı yakalıyor |
| M5.3 | Kalıcı stil: Stylo snapshot'ları, yeniden stil ipuçları, hasar → bitler | Bir sınıf değişikliği yalnızca etkilenen elemanları stilliyor; `:has()` vakası çalışıyor |
| M5.4 | Kalıcı layout: Taffy önbelleği, kirlenme yukarı, sınırlar, erken kesme | B1 ve B5 tabana göre ölçülmüş; sınır testi |
| M5.5 | Kalıcı metin: şekillendirme önbelleği | Bir harf yalnızca kendi paragrafını şekillendiriyor |
| M5.6 | Display list parçaları, hasar bölgesi, `RenderBackend`, kısmi sunum | B6 ölçülmüş; tam kare ve kısmi kare piksel piksel aynı (altın test) |
| M5.7 | Erişilebilirlik: AccessKit, tembel ağaç | Bir ekran okuyucu form etiketlerini okuyor; B8 |
| M5.8 | Formlar, imleç, seçim, pano, IME, odak; `<details>`, `<dialog>`, `popover`, `commandfor` | Taşın mevcut kabulü |

Kısmi kare ile tam karenin aynı pikselleri vermesi M5.6'nın muhafızıdır.
Artımlılığın en sinsi hatası, ekranda unutulmuş eski bir parçadır. Her
artımlı testin bir de tam yeniden hesapla karşılaştırması olur: aynı
mutasyon dizisi iki yoldan geçer ve display list'ler eşit olmalıdır. Bu,
M4'ün `Mutation` fuzz'ına bağlanır: fuzz her diziyi iki yoldan da çalıştırır.

## 6. Erk'e özgü olan

"İlk" ya da "en hızlı" iddiası yok. Savunulabilir olanlar şunlar:

- **Tek değişiklik yolu.** Host, Python, Go ve JS bağlamaları, kullanıcı
  olayları, hepsi aynı `Mutation` journal'ından geçer. Çekirdekte ikinci bir
  reaktif sistem yok.
- **Tek invalidation sözlüğü, nedeniyle birlikte.** Stil, metin, layout,
  boyama ve erişilebilirlik aynı bitlerle, aynı yayılmayla. Her kirlenmenin
  nedeni DevTools'ta görünür.
- **Containment kendiliğinden.** Sınırlar hesaplanmış stilden çıkarılır,
  erken kesme her sabit boyutlu kutuda kendiliğinden çalışır. CSS
  `contain` bir hızlandırma ipucu değil, sınırın açık hâlidir.
- **Kendi tabanına karşı ölçülmüş.** M2'nin bilerek kaba tam yeniden hesabı
  her artımlı yolun hem hız tabanı hem doğruluk kâhini: iki yol aynı display
  list'i vermek zorunda.

## 7. Kapsam dışı (M5)

| Ne | Neden |
|---|---|
| Genel iş grafiği, düğüm düzeyinde paralel layout | Ölçülmeden karmaşıklık; M9'da kapı (§3.9) |
| Motor içi sinyaller | Durum host'un; sinyaller bağlamalarda (§3.10) |
| DOM'un paylaşılan snapshot'ı | Sözleşmenin düz veri kuralı; raster'a display list gider (§2) |
| Kendi seçici invalidation'ımız | Stylo'nun işi (§3.4) |
| `content-visibility` | css-support "Later"; sınırlar ve erken kesme önce |

## 8. Kaynaklar ve ön çalışmalar

Bu alanda kanıtlanmış teknikler; Erk bunları yeniden icat etmez, kendi
modeline bağlar:

- Stylo'nun yeniden stil ipuçları, eleman snapshot'ları ve göreli seçici
  (`:has()`) invalidation'ı (Firefox).
- Flutter'ın relayout boundary'si: boyutu ebeveyninden gelen kutu yayılmayı
  keser.
- Blink LayoutNG'nin kısıt alanı anahtarlı layout önbelleği; Taffy'nin
  önbelleği aynı fikrin küçük bir hâli.
- Masonry/Xilem'in geçiş istekleri (layout, boyama, erişilebilirlik
  bayrakları yukarı yayılır).
- Salsa'nın erken kesmesi: yeniden hesaplanan değer aynıysa bağımlılar
  yeniden hesaplanmaz.
- AccessKit'in etkinleşmeyle kurulan ağacı; `softbuffer`'ın hasarlı sunumu.

## 9. Açık sorular (M5'in ilk adımında kapanır)

- Stylo snapshot'ı için `TElement` ve `ElementSnapshot` uygulaması
  `erk-style`'ın beş `unsafe fn` imzalı yüzeyini değiştiriyor mu?
  Değiştiriyorsa muhafız ve gerekçe aynı PR'da güncellenir.
- Taffy önbelleğini kare arasında korumak: `layout::build` her kare yan
  tabloyu kuruyor. Kalıcı yan tablo, M4'ün arena silmesi ve nesilleriyle
  nasıl eşleşir?
- `vello_hybrid`'de kısmi sunum: makas dikdörtgeni mi, döşeme önbelleği mi
  (M9 ile sınır)?
- `contain` css-support.md'ye hangi değerlerle girer (`size`, `layout`,
  `paint`) ve hangi testle?

## 10. Nihai kararlar (özet)

1. Tek değişiklik yolu: transaction → `MutationJournal` (birleştirme) →
   kare sınırında uygulama. Sinyaller bağlamalarda, `Mutation` üretir.
2. Tek invalidation sözlüğü, yön bitlerin içinde; nedenler journal'ın halka
   tamponunda, `inspect` özelliğinin arkasında.
3. Stil invalidation'ı Stylo'nun; layout yeniden kullanımı Taffy'nin
   önbelleği, üstüne hesaplanmış stilden sınırlar ve erken kesme.
4. Cebir `clamp`'siz: boş girdi, tekrar uygulama, monotonluk, birleşim
   üzerine dağılma ve O(d) yakınsama özellik testleriyle.
5. Kutu başına display list parçaları, düğüm → parça yan tablosu, arka
   uçtan bağımsız hasar bölgesi; döşeme boyutu ölçümle.
6. AccessKit ağacı etkinleşmeyle kurulur, sonra yalnızca kirliler.
7. Genel iş grafiği yok; M9'da ölçüm kapısı, alt ağaç birimli; DAG olarak
   kurulur ve döngü tespiti kabul testidir.
8. `erk-invalidation` yalnızca `erk-dom`'a bağımlı; düğüme bağlı veri yan
   tablolarda.
9. Her ölçüm M2'nin tam yeniden hesabına karşı; her artımlı yol display
   list eşitliğiyle tam yeniden hesaba karşı doğrulanır.
