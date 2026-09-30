# Erk Engine — P1: Gömülü masaüstü UI motoru

- **Tarih:** 2026-09-30
- **Önceki tasarım:** [p0-architecture.md](p0-architecture.md). Render hattı,
  DOM bellek modeli ve test stratejisi geçerli kalır. Tarayıcıya özgü
  bölümleri (§2.1 süreç izolasyonu, §5.3 JS sahipliği, §7 ağ, §8 JS) bu
  belge geçersiz kılar.
- **Sözleşme:** [p1-contract.md](p1-contract.md)
- **Yol haritası:** [roadmap.md](../plans/roadmap.md)

---

## 1. Karar

Erk bir tarayıcı değil, **gömülü bir HTML/CSS masaüstü UI motoru**dur.
Host uygulama (önce Rust, sonra C-ABI üzerinden Python) DOM'u sürer, Erk
stil, layout ve boyamayı yapar, kullanıcı olaylarını host'a bildirir.
Motorda betik dili yoktur.

```
host uygulama (Rust, Python, C)      iş mantığı, durum, dosyalar
        │   ▲
        │   │  NodeId'ler, toplu değişiklikler, olaylar
        ▼   │
erk (Rust API) ── erk-ffi (C-ABI, erk.h)
        │
çekirdek: erk-dom · erk-style · erk-renderer     G/Ç yok, saat yok, ortam yok
        │
pencere: winit + softbuffer (M2'de vello_hybrid)
```

**Neden:** tam bir tarayıcı tek kişilik bir proje için bitmeyen bir yüktür.
JS motoru ve DOM-GC mimarisi, Fetch, çerezler, kum havuzu ve site izolasyonu
her biri çok yıllık işlerdir. Masaüstü UI için bunların hiçbiri gerekmez. M0'da
kurulan her şey ise yeni hedefe doğrudan yarıyor: arena DOM, Stylo, Taffy,
Parley, vello ve yalnızca düz veri mesajlarla konuşan renderer.

**Kullanıcının kararları:** entegrasyon süreç içi bir kütüphane (önce Rust API,
sonra C-ABI; ayrı süreç modu ancak ihtiyaç olursa); ilk dil bağlaması Python.

## 2. İlkeler

1. **Sıfır JS, sıfır GC.** `<script>` ayrıştırılır ama hiçbir zaman çalışmaz.
2. **DOM arenanındır.** Dış dünya düğümlere yalnızca `NodeId` ile başvurur
   (u32 indeks + u32 nesil; C tarafında opak `uint64_t`). Silinmiş bir
   düğümün id'si çökme değil hata kodu üretir.
3. **Tek yönlü akış.** Host durumu değiştirir → toplu değişiklik (`Mutation`)
   Erk'e gider → Erk DOM'u günceller → yeni kare → olaylar host'a döner.
4. **Çekirdek saftır.** `erk-dom`, `erk-style` ve `erk-renderer` dosya, ağ,
   süreç, ortam değişkeni ve saat kullanmaz. Kaynaklar (CSS `url()`, `<img>`)
   host'un verdiği bir callback'ten, zaman host'un verdiği `now_ns`'ten,
   yapılandırma API parametrelerinden gelir. Bu hem güvenlik (içerikteki
   `url("file:///etc/passwd")` hiçbir şey okuyamaz) hem belirleyicilik
   (testler zamanı ve kaynakları kendisi sürer) içindir. CI'da zorlanır
   (`.github/scripts/check-core-io.sh`).
5. **`unsafe` yalnızca adıyla listelenmiş crate'lerde.** Bugün `erk-style`;
   M3'te `erk-ffi` (C-ABI ham işaretçi ister; öznitelik edition 2024'te
   `#[unsafe(no_mangle)]`). Çekirdek `forbid`'de kalır.
6. **Sözleşme koddan önce.** ABI, iş parçacığı modeli, bellek ve callback
   ömrü M1'den önce M0.5'te yazılır. M1 ve M2'nin Rust çekirdeği bu
   sözleşmeyi bozmayacak biçimde yazılır: sınırda düz veri, winit, `Arc` ve
   Rust closure'ları yok.

## 3. Gelen plan önerisinin değerlendirmesi

Yön değişikliği, dışarıdan gelen bir plan önerisi ve ona yapılan iki
eleştiriyle şekillendi. Alınanlar ve düzeltilenler:

| Öneri | Karar |
|---|---|
| Sıfır JS, DOM arenada, `NodeId` ile erişim, tek yönlü akış | Alındı (§2) |
| `extern "C"` ve `#[no_mangle]` ile C-API | Alındı, düzeltmeyle: `unsafe` ister, `erk-ffi` adıyla listelenmiş istisna olur; `erk.h` cbindgen ile üretilir ve CI depodaki kopyayla karşılaştırır |
| C-ABI'yi M3'te tasarlamak | Düzeltildi: sözleşme M0.5'te, M1'den önce |
| Hem C-ABI hem IPC broker | Yalnızca süreç içi kütüphane; mesajlar serileştirilebilir kaldığı için ayrı süreç sonradan eklenebilir |
| Tıklamanın `data-erk-action` özniteliğiyle bildirilmesi | Reddedildi: host belirli düğümlere abone olur, olaylar DOM'un capture/bubble alt kümesiyle dağılır. Öznitelik, içeriği bağlama mantığıyla karıştırır |
| Hit-test için QuadTree | Ertelendi: layout kutuları boyama sırasının tersinden gezilir; ölçüm yapıyı gerektirirse eklenir |
| M1'de float ve tablolar | Çıkarıldı: masaüstü UI'ı flex ile kurulur; float CSS'in en çok köşe durumu barındıran yeri. M1 düzeni: block, inline metin, flex, absolute |
| "CSS destekliyoruz" | Düzeltildi: [css-support.md](../css-support.md) neyin desteklendiğini ve neyin **hiç** planlanmadığını listeler |
| Dosya erişimi motorda | Düzeltildi: G/Ç yalnızca host'ta (§2.4) |
| Fuzzing ve metrikler | Alındı: M1 kabulünde. "< 5 MB ikili, < 15 ms ilk kare" hedefleri doğrulanmadı; bütçe M1'in başında M0'ın ölçülen taban çizgisinden konur |
| DevTools sunucusu (TCP/WebSocket) | Düzeltildi: varsayılan olarak ağ dinlenmez; önce süreç içi, Erk ile çizilen bir inspector |
| M2'de `:hover` | Alındı, notla: M5'e kadar her durum değişikliği tam yeniden stil, layout ve boyama ister; M2 ölçümleri performans iddiası değil M5'in kıyas tabanıdır |
| Metinde olmayanlar | Eklendi: IME (Windows TSF), seçim, pano, odak gezinmesi, AccessKit (M5), sistem fontları ve fallback, HiDPI, kenarlık/yuvarlak köşe/görüntü |

Sonra gelen bir "ana plan" önerisinin M0.5 sonrası değerlendirmesi:

| Öneri | Karar |
|---|---|
| Yapılandırılmış kaynak API'si (`ResourceRequest`, `ResourceResponse`) | Alındı: istek türü (görüntü, stil sayfası, font) ve yanıt MIME'i sözleşmeye girdi (p1-contract §6). HTTP tarzı `status: u16` alınmadı: ağ yok, durum `ErkStatus` |
| `erk-dom` düğümüne şimdiden `aria_role`/`aria_label` alanı | Reddedildi: bilgi zaten öznitelik olarak (`role`, `aria-*`) saklanıyor, boş alanlar ölü kod olurdu. AccessKit M5'te özniteliklerden kurulur (p1-contract §6.1) |
| M1'i mikro adımlara bölmek | Alındı: M1.0–M1.7. İlk iki adım (tek satır metin, satır kırma) M0'da zaten var ve testli |
| Float ve tablo görülünce sessizce `display: none` | Reddedildi: içeriği gizler. Float `none` gibi dizilir; tablo için tablo algoritması yok (css-support.md) |
| "< 5 MB" ikili | Değişmedi: bütçe M1.0'da ölçülen tabandan konur |
| Sayaç uygulaması | Alındı: M4'ün ilk demosu; kabul ölçütü daha güçlü olan TodoMVC olarak kalır |

Geliştirici araçları önerisinin (F12 ile açılan, Erk ile yazılmış DevTools)
değerlendirmesi:

| Öneri | Karar |
|---|---|
| DevTools'u M7'ye kadar bekletmemek, parça parça kurmak | Alındı; her parça dayandığı altyapının taşına: `inspect_at` ve vurgu M2 (hit-test), denetim sorguları ve aşama süreleri M3 (API), canlı CSS M5 (artımlı stil), DevTools uygulaması M7 |
| Önerideki M2–M7 numaralandırması | Uyarlandı: önerinin taşları bu projeninkilerle örtüşmüyor (örneğin API M3'te geliyor, sorgular ondan önce olamaz) |
| Performance paneli için aşama süreleri | Alındı, düzeltmeyle: çekirdek saat okumaz; süreleri aşamaları çağıran `erk` crate'i ölçer (p1-contract §8.1) |
| Console | Alındı, JavaScript konsolu olarak değil: host'un logları ve Erk'in uyarıları (`ErkLogFn`) |
| Network paneli | "Resources" olarak alındı: Erk'in host'tan istediği kaynaklar. Motorun ağı yok; host'un ağ trafiği ancak host beslerse görünür |
| JSON-RPC / WebSocket DevTools protokolü | Ertelendi: önce süreç içi ikinci pencere. Motor varsayılan olarak port dinlemez; uzak DevTools yalnızca açıkça etkinleştirilirse, yerel bir kanaldan |
| DevTools'u Erk ile yazmak | Alındı: M7'nin kabul ölçütü, motoru kendi aracıyla sınar |

Ürün yol haritası önerilerinin (M0–M21 listesi ve düzeltmeleri)
değerlendirmesi:

| Öneri | Karar |
|---|---|
| Çekirdek yalnızca UI; tepsi, diyalog, bildirim, kısayol, güncelleme host'ta | Alındı: ilke; `erk new` şablonları bunları host kodu olarak getirir (M8) |
| Surface API (host'un GPU çizimi) | Alındı (M10), düzeltmeyle: bir yüzeyin tek çizicisi Erk; host bölgeye callback ya da dokuyla çizer (p1-contract §8.2). İşaretleme özniteliği değil API |
| Başka `ErkApp`'in `NodeId`'si | Gerçek bir açık; ilk çözüm (neslin üst 8 biti uygulama etiketi) sonra değiştirildi: iç temsil 32 bit indeks + 32 bit nesil olarak kalır, dış id uygulamaya özel bir anahtarla karıştırılır (p1-contract §2). Etiket düzeni nesli 24 bite indirip uzun süre açık kalan uygulamada sızıntıya, 256 etiketin yeniden kullanımına ve okunabilir bitlere yol açıyordu. Karıştırma güvenlik sınırı değil, ad alanı ayrımı; yakalama olasılıksal |
| ABI'yi M0.5'te dondurmamak | Alındı: ABI v0.1; 1.0'a (M8) kadar kırıcı değişiklik hakkı saklı |
| Kirlenme bitlerini ayırmak, stil geçersizleştirmesinin CSS bağımlılıklarına bakması | Alındı (M5): stil, layout, boyama ve metin ayrı; `.parent:hover .child` gibi bağımlılıkları Stylo'nun yeniden stil ipuçları ve snapshot'ları taşır |
| Test stratejisi | Alındı: [p1-testing.md](p1-testing.md). Ayrı bir taş (M0.6) yerine sürekli altyapı; ilk yeni parçası Chrome'la geometri karşılaştırması |
| Chrome test kümesini büyütmek, geometri farkını tutmak | Alındı ve uygulandı: her referans sayfası için kutuların Chrome'la karşılaştırılması |
| "Chrome ile piksel farkı sıfır" varsayımı | Düzeltildi: kutular ve arka planlar örtüşüyor, metin kenar yumuşatması ve hinting farklı; metinle dolu sayfaların içerik skoru bu yüzden ~%48. Kesin ölçü geometri karşılaştırması |
| Go bağlaması, Nexus Mail kıyası | Alındı: Go M6'da; Wails + React ile Go + Erk kıyası M8'in kabulünde |
| SVG, video | Alındı (M11): resvg → vello; video karesi surface dokusuna; ses host'ta |
| Bileşenler, Tailwind | Alındı (M12): yerel kontroller motorda, kalıplar HTML/CSS ile, karmaşık bileşenler host kütüphanesi; Tailwind CSS değişkenleri ister (Stylo'da var, testli) |
| Grid'i float ve tablodan önce | Zaten öyle: grid "Later", float ve tablo "Not planned" |
| Animasyon veri modelini erken hazırlamak | Zaten var: Stylo `transition`, `animation`, `@keyframes`'i ayrıştırıyor; çalışma zamanı M9 |
| Erişilebilirlik modelini erken düşünmek | Alındı: rol, ad, değer ve durum özniteliklerden türetilir (p1-contract §6.1); türetme kuralları M3'teki denetim sorgularıyla birlikte yazılır, işletim sistemi bağlantısı M5 |
| M0–M21 diye yeniden numaralamak | Alınmadı: numaraları değiştirmek her belgeyi ve commit geçmişini bozar; yeni işler M10–M12 olarak eklendi |
| "Özelliklerin %85'i JS'siz yapılabilir" | Alınmadı: ölçülebilir değil |

Tarayıcı hedefi için daha önce gelen öneriler (M4'te JS/DOM-GC sınır tablosu,
M6'da Fetch ve `cookie_store`) yeni hedefte konu dışı. M1 için olanların
doğrulanmış hali geçerli:

- IFC, Blitz 0.3.0-beta.2'nin `layout/inline.rs` ve `construct.rs`
  dosyalarından uyarlanır (MIT OR Apache-2.0, dosya başına kaynak yorumu).
  Blitz'in tek `unsafe`'i calc çözümlemesi; onun yerini Erk'in `CalcTable`'ı
  zaten tutuyor.
- Anonim blok kutularının yeri (DOM mu, layout yan tablosu mu) portun ilk
  adımında kararlaştırılır.
- Servo'nun layout kodu MPL-2.0: yalnızca okunur, kopyalanmaz.
- "İki haftalık zaman kutusu" süre tahmini yasağıyla çelişir; karar kapısı
  bir test sonucuna bağlanır.

## 4. Konumlandırma

Bu alanda boş bir yer yok; farkı dürüst yazmak gerekiyor.

| Proje | Ne | Erk'ten farkı |
|---|---|---|
| Sciter | HTML/CSS gömülü UI motoru, C-API, çok dilli bağlamalar | Kapalı kaynak; kendi betik dili var |
| Blitz / Dioxus Native | Rust, JS'siz HTML/CSS renderer | API yalnızca Rust'tan; Erk'in ilk dili de Rust ama C-ABI ve Python hedefte |
| Ultralight | WebKit tabanlı gömülü motor | Kapalı kaynak, JS var |
| Tauri, Electron | Sistem webview'ı ya da Chromium | JS ile çalışır; Electron büyük, Tauri platformun webview'ına bağımlı |
| Slint, egui, Qt | Yerel UI araç takımları | HTML/CSS değil |

Erk'in iddiası: standart HTML/CSS'in açıkça sınırlanmış bir alt kümesi, sıfır
JS, kararlı bir C-ABI ve Python, her makinede aynı çizim, küçük ikili. Boyut
ve bellek iddiaları ölçülmeden yazılmaz.

## 5. Mimari

- **Çekirdek:** `erk-dom` (arena, html5ever), `erk-style` (Stylo),
  `erk-renderer` (layout, display list, boyama, renderer iş parçacığı).
  Değişmeden kalır; M4'te arena silme, M5'te kalıcı stil ve layout kazanır.
- **`erk` (M3):** idiomatik Rust API'si. `App`, düğüm tutamakları, toplu
  değişiklikler, olay abonelikleri.
- **`erk-ffi` (M3):** aynı API'nin C-ABI yüzü; `erk.h` üretilir. Adıyla
  listelenmiş `unsafe` istisnası.
- **`erk-shell`:** bugün demo host ve kabuk (`erk <dosya.html>`,
  `--screenshot`). M3'te `erk`'in ilk kullanıcısına dönüşür.
- **Kaynak sağlayıcı:** host'un callback'i. Demo kabuğun sağlayıcısı bir kök
  dizinle ve `memory://` ile sınırlıdır; kökün dışı ve başka şemalar
  reddedilir.
- **Mesajlar:** bugünkü `ToRenderer`/`FromRenderer` düz veri kuralı genişler:
  `Mutate(Vec<Mutation>)` ve olay mesajları aynı dosyada, aynı muhafızla.

`erk-network` kaldırıldı: motor ağa hiç erişmez; bir host ağdan veri alırsa
bunu kendisi yapar ve DOM'a değişiklik olarak verir.

## 6. Kapsam dışı

| Ne | Neden |
|---|---|
| JavaScript ve her türlü betik | Motorun ilkesi (§2.1) |
| Ağ, HTTP, Fetch, çerezler | Host'un işi |
| Kum havuzu, çoklu süreç | İçerik host'un kendisi; ihtiyaç olursa mesaj disiplini sayesinde sonradan eklenir |
| Float, tablo düzeni, multi-column, print/paged media | [css-support.md](../css-support.md) "Not planned" |
| WebExtensions, medya ve DRM, WebRTC | Tarayıcı işleri |
| Mobil | Hedef masaüstü |
