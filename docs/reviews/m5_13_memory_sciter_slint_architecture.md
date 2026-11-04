# Erk Engine - M5.13 Bellek Diyetinde Sciter ve Slint Düzeyine Ulaşma Mimari Raporu

**Tarih:** 2026-11-04  
**Kapsam:** M5.13 (Bellek) - GPU Açıkken 344 MB CPU RAM Tüketiminin Analizi ve Ultra-Hafiflik Stratejileri  
**İnceleyen:** Antigravity (Kod Gözden Geçirici / Code Reviewer)

---

## 1. Giriş ve Sorunun Tanımı

> [!NOTE]
> **Uygulama ve İnceleme Notu:** Bu rapordaki maddeler birer araştırma önerisi ve hipotezdir; DX12 backend'inin hemen değiştirilmesi gerekmez. 344 MB'lık tüketimin ne kadarının sürücüden, ne kadarının wgpu havuzlarından veya Erk'in iç önbelleklerinden geldiği M5.13'ün ilk adımı olan bellek profilinde (`dhat` / B16) ölçülmeden kesin kabul edilmemelidir. Geliştirmeyi yapacak yapay zekâ / mühendis, önce ölçüm verilerini toplamalı, performansı veya CSS uyumluluğunu riske atmadan kendi kararını vermelidir.

Erk Engine, `m2-demo.html` gibi küçük ve statik bir sayfada test edildiğinde şu bellek ölçümleri alınmıştır:
* **CPU Raster (`--cpu` / `vello_cpu`):** 23,7 MB Özel Bellek (Private Bytes) / 40,8 MB Çalışma Kümesi (Working Set)
* **GPU Raster (`vello_hybrid` / `wgpu` DX12):** **344 MB Özel Bellek** / 194 MB Çalışma Kümesi
* **10.000 Düğümlü Sayfa (`erk-10000.html`):** CPU'da 472 MB (eleman başına ~15 KB), GPU'da 793 MB

Ortaya çıkan yaklaşık **320 MB'lık fark doğrudan VRAM değil, süreç CPU RAM'i (Private Bytes)** tüketimidir. Erk'in Chromium gibi ağır gömülü motorlardan ayrışıp **Sciter (~10–15 MB)** ve **Slint (~15–25 MB)** seviyesinde hafif bir motor olabilmesi için bu farkın kaynakları ve çözüm rotası aşağıda analiz edilmiştir.

---

## 2. Bellek Tüketiminin Katman Dağılımı

GPU açıkken gözlemlenen 344 MB'lık tüketim 3 ana katmandan oluşmaktadır:

```
┌────────────────────────────────────────────────────────────────────────┐
│ 1. OS & GPU Sürücüsü Katmanı (Windows DX12 / UMD) : ~100 – 180 MB       │
│ 2. wgpu Altyapısı (Descriptor Heaps & Pools)       : ~40 – 70 MB        │
│ 3. Erk & vello_hybrid CPU Önbellekleri / Fallback : ~60 – 100 MB       │
└────────────────────────────────────────────────────────────────────────┘
```

1. **Windows DX12 Sürücü Yığını:** Windows'ta `ID3D12Device` başlatıldığı an donanım üreticisinin kullanıcı modu sürücüsü (NVIDIA, Intel, AMD DLL'leri) ve DX12 Agility SDK; komut kuyrukları, shader JIT derleyicisi ve GPU sanal adres sayfa tabloları için peşinen devasa bir sanal ve taahhütlü (committed) bellek bloğu rezerve eder.
2. **`wgpu` Varsayılan Limitleri:** `DeviceDescriptor::default()` kullanımı, 2D arayüz motoru için gereksiz olan devasa CBV/SRV/UAV descriptor heap'lerini ve staging havuzlarını peşinen tahsis eder.
3. **Eşzamanlı CPU Fallback Temizliği:** Soğuk başlangıçta CPU rasterizer'ın ayırdığı `cpu_frame` pikselleri GPU hazır olunca bırakılsa dahi, Windows Heap Yöneticisi bu sayfaları işletim sistemine hemen iade (decommit) etmez.

---

## 3. Sciter ve Slint Modeline Yaklaşma Stratejisi

### 3.1 Hamle 1: Grafik Hattı ve Sürücü Katmanı (Tasarruf: ~150–200 MB)
* **Sciter Modeli:** Sciter Windows'ta Direct2D/DirectWrite kullanır ve işletim sisteminin halihazırda yüklü paylaşımlı servislerinden faydalanarak sürücü yükünü ~10 MB seviyesinde tutar.
* **Erk Çözümü:**
  * `wgpu` tarafında Windows için D3D11 arka ucu alternatifi değerlendirilmelidir (D3D11 sürücüleri DX12 gibi agresif peşin havuzlar açmaz).
  * `wgpu::DeviceDescriptor` içine 2D motoruna özel sıkı `wgpu::Limits` (ör. `downlevel_webgl2_defaults`) verilerek havuzlar küçültülmelidir.
  * `vello_cpu` (CPU rasterizer) şu an bile 23,7 MB ile çalışmaktadır. Masaüstü UI senaryolarında GPU zorunluluğu esnetilerek, SIMD + döşeme hasar takipli CPU modu birinci sınıf ultra-hafif seçenek olarak sunulmalıdır.
  * Linux (Mesa Vulkan) ortamında sürücü tabanının ~25 MB'a gerilediği doğrulanmalıdır.

### 3.2 Hamle 2: CPU Fallback Decommit ve Bellek Ayırıcısı
* GPU devraldığında boşa çıkan CPU tamponlarının (`self.cpu_frame = None`) işletim sistemine anında iade edilmesi sağlanmalıdır.
* Windows CRT malloc yerine `mimalloc` veya `snmalloc` gibi decommit davranışı güçlü global ayırıcıların kalıcı belleğe etkisi B16 senaryosu ile ölçülmelidir.

### 3.3 Hamle 3: Stylo Stil Paylaşımı (Stil Katmanı)
* 10 bin kartlık stres sayfasında aynı stillere sahip elemanların `Arc<ComputedValues>`'u paylaşması (`StyleSharingCache` etkinliği) doğrulanmalı; eleman başına stil yükü asgariye indirilmelidir.

### 3.4 Hamle 4: Metin ve Paragraf Verilerini Tekilleştirme
* `ShapeCache` (kırılmamış saf düzen) ile `ShapedText` (kırılmış sonuç) arasındaki çift kopya tekilleştirilerek dilim (`slice`) modeline geçilmelidir.

### 3.5 Hamle 5: DOM Düğüm Boyutu ve String Interning
* Düğümlerdeki öznitelik dizgileri atom/interning havuzuna alınmalı; eleman başına 15 KB maliyet hedeflenen 3–4 KB bandına çekilmelidir.

---

## 4. Sonuç ve Hedef Tablosu

| Metrik | Mevcut Erk | Alınacak Tedbirler Sonrası | Sciter / Slint |
| :--- | :--- | :--- | :--- |
| **Küçük Sayfa (CPU)** | 23.7 MB | **12 – 14 MB** | 10 – 15 MB |
| **Küçük Sayfa (GPU)** | 344 MB (DX12) | **40 – 60 MB** (D3D11 / Vulkan / Sıkı Limits) | 20 – 35 MB |
| **10.000 Düğüm** | 472 MB | **100 – 140 MB** | 60 – 90 MB |

Bu mimari analiz ve çözüm rotası [`docs/plans/m5-incremental-forms.md`](file:///c:/Users/ismet.kabatepe/erk-engine-browser/docs/plans/m5-incremental-forms.md#L335) içerisindeki M5.13 bölümüne resmi plan olarak entegre edilmiştir.
