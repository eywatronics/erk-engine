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
