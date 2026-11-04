# Erk Engine - M5.13 Bellek Diyetinde Doğrulanmış Mimari Yol Haritası

**Tarih:** 2026-11-04  
**Kapsam:** M5.13 (Bellek) - 344 MB Özel Bellek ve 10.000 Düğümlü Sayfa Analizi  
**İnceleyen:** Antigravity (Kod Gözden Geçirici / Code Reviewer)

---

## 1. Giriş ve Sorunun Tanımı

Erk Engine, `m2-demo.html` gibi küçük ve statik bir sayfada test edildiğinde şu bellek ölçümleri alınmıştır:
* **CPU Raster (`--cpu` / `vello_cpu`):** 23,7 MB Özel Bellek (Private Bytes) / 40,8 MB Çalışma Kümesi (Working Set)
* **GPU Raster (`vello_hybrid` / `wgpu` DX12):** **344 MB Özel Bellek** / 194 MB Çalışma Kümesi
* **10.000 Düğümlü Sayfa (`erk-10000.html`):** CPU'da 472 MB (eleman başına ~15 KB), GPU'da 793 MB

Ortaya çıkan yaklaşık **320 MB'lık fark doğrudan VRAM değil, süreç CPU RAM'i (Private Bytes)** tüketimidir. 

Ancak mühendislik disiplini açısından şu ayrım kritiktir:
1. **Private Bytes vs. Working Set:** 344 MB Private Bytes, işletim sistemine taahhüt edilmiş (committed) sanal adres alanıdır; bu 344 MB'ın tamamının fiziksel RAM'de aktif tutulduğu (Working Set) anlamına gelmez.
2. **Kanıtlanmamış Varsayımlardan Kaçınma:** 344 MB'ın ne kadarının sürücüden, ne kadarının wgpu havuzlarından veya Erk'in iç tamponlarından kaynaklandığı B16 profili çıkarılmadan kesin rakamlarla iddia edilemez.
3. **Standart wgpu Sınırları:** Güncel `wgpu` ekosisteminde standart Windows backend'i D3D12 ve Vulkan'dır; D3D11 seçeneği wgpu'da doğrudan mevcut değildir. Dolayısıyla çözüm backend değiştirmek değil, mevcut mimarideki veri tekrarlarını ve gereksiz kaynakları tespit etmektir.

---

## 2. Bellek Optimizasyonunda Üç Temel İlke

1. **Önce Ölç, Sonra Optimize Et:** 344 MB'ın kaynağını varsaymak yerine aşama aşama profillemek (`dhat`, ETW/VMMap).
2. **Mevcut Altyapıyı Değiştirmeden Önce Veri Tekrarlarını Bul:** Stylo veya Parley gibi devasa kütüphaneleri yeniden yazmak yerine, önbelleklerde çift tutulan verileri tekilleştirmek.
3. **Performans (FPS) Pahasına Diyet Yapma:** Erk'in en büyük gücü artımlı render (incremental rendering) ile kazandığı 60 FPS hızdır. Bellek kısıtlamaları p95 kare süresini veya CSS uyumluluğunu bozmamalıdır.

---

## 3. M5.13 Uygulama Yol Haritası (M5.13-A ➔ F)

### M5.13-A: B16 Bellek Profilini Çıkar (Temel Adım)
* CPU ve GPU yollarında belleğin nereye gittiği aşamalara ayrılır: DOM, Stylo, Taffy, Parley, Display List, Raster, wgpu ve Sürücü.
* Private Bytes ile Working Set ayrımı gözetilerek; açılış, kaydırma ve boşta bekleme durumları ayrı ayrı ölçülür.

### M5.13-B: GPU Bellek Maliyetini İncele (Renderer Değiştirmeden)
* Backend değiştirmeye kalkışmadan önce; wgpu staging alanları, render kaynakları, CPU fallback tamponlarının (`self.cpu_frame`) bırakılma/decommit anı ve aynı Windows donanımında DX12 vs. Vulkan karşılaştırması yapılır.

### M5.13-C: Parley Veri Tekilleştirmesi
* Gerçekten çift tutulan metin verileri (`ShapeCache`'teki kırılmamış düzen ve `ShapedText`'teki kırılmış sonuç) belirlenir. Performans düşürülmeden tekilleştirme uygulanır.

### M5.13-D: DOM ve Öznitelik Kompaktlaştırma
* Tekrarlanan string'ler için interning, `SmallVec` kullanımı ve gereksiz heap tahsislerinin azaltılması. 10.000 düğümlü sayfada (~50.000 node) düğüm başına maliyet optimize edilir.

### M5.13-E: Stylo Denetimi
* Stylo'nun kendi `StyleSharingCache` mekanizmasının etkinliği `Arc<ComputedValues>` tekil sayaçlarıyla doğrulanır. Yeni bir katman yazılmadan önce mevcut önbellekleme davranışı ölçülür.

### M5.13-F: CI Bütçesi ve Regresyon Muhafızı
* Her optimizasyonun bellek kazancı ve FPS üzerindeki etkisi ölçülür; p95 kare süresi veya doğruluk kâhini bozulmadan `.github/memory-budget.txt` tavanı kilitlenir.

---

## 4. Araştırma Hedefleri

| Senaryo | Mevcut Ölçüm | Araştırma Hedefi | Değerlendirme |
| :--- | :--- | :--- | :--- |
| **Küçük Sayfa (CPU)** | 23,7 MB | **12 – 18 MB** | Makul optimizasyon hedefi |
| **10.000 Düğüm** | 472 MB | **100 – 150 MB** | Ciddi bir hedef; birden çok aşamanın optimize edilmesini gerektirir |
| **Küçük Sayfa (GPU)** | 344 MB | **40 – 80 MB** | Kaynağı B16 ile doğrulandıktan sonra değerlendirilecek hedef |

Bu plan, [`docs/plans/m5-incremental-forms.md`](file:///c:/Users/ismet.kabatepe/erk-engine-browser/docs/plans/m5-incremental-forms.md#L335) dosyasındaki resmi M5.13 bölümü ile birebir uyumludur.
