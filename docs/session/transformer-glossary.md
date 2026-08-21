# Transformer Glossary

## KVAR

***KVAR*** stands for **Kilovolt-Amperereactive**. It is the unit used to measure **reactive power** in an alternating current (AC) electrical system. It represents the power that does not do real work, but is needed to run machines with magnets. [[1](https://blog.se.com/infrastructure-and-grid/power-management-metering-monitoring-power-quality/2021/04/16/kvar-kvar-or-kvar/), [2](https://www.electricsaver1200.com/what-is-kvar/), [3](https://www.youtube.com/shorts/l0tmYP5Yi4E)]

### How KVAR Works

- **Real Power (KW):** This power does the actual work, like turning a motor, making heat, or lighting a bulb.
- **Reactive Power (KVAR):** This power flows back and forth. It builds and keeps magnetic fields in things like motors, transformers, and pumps.
- **Apparent Power (KVA):** This is the total power pulled from the power lines, made up of both KW and KVAR combined. [[1](https://powerelectrics.com/blog/the-difference-between-kva-and-kvar/), [2](https://www.iammeter.com/blog/reactive-power-kvar-kvarh-pf), [3](https://www.youtube.com/watch?v=OECFsfKxyYo&t=213), [4](https://strongpowerelectric.com/kva-vs-kvar-and-the-difference/), [5](https://www.youtube.com/shorts/l0tmYP5Yi4E)]

### Why KVAR Matters

- **No Direct Work:** KVAR does not spin a fan blade or heat a room by itself, but machines cannot run without it. [[1](https://powerelectrics.com/blog/the-difference-between-kva-and-kvar/), [2](https://www.linkedin.com/posts/naeem-abbasi-76325156_what-is-kvar-there-are-two-types-of-power-activity-7272903050007105536-p28b), [3](https://www.youtube.com/shorts/l0tmYP5Yi4E)]
- **System Strain:** Too much KVAR makes wires carry extra current, which wastes energy and heats up cables. [[1](https://elintacharge.com/glossary/reactive-power-kvar/), [2](https://blog.se.com/infrastructure-and-grid/power-management-metering-monitoring-power-quality/2021/04/16/kvar-kvar-or-kvar/), [3](https://bchindia.com/blogs/kvar-capacitors-explained-for-industrial-power-systems/), [4](https://www.linkedin.com/posts/naeem-abbasi-76325156_what-is-kvar-there-are-two-types-of-power-activity-7272903050007105536-p28b)]
- **Power Factor:** High KVAR lowers your power efficiency (the power factor). Power companies often charge extra money to factories or large buildings if their KVAR use is too high. [[1](https://bchindia.com/blogs/kvar-capacitors-explained-for-industrial-power-systems/), [2](https://blog.se.com/infrastructure-and-grid/power-management-metering-monitoring-power-quality/2021/04/16/kvar-kvar-or-kvar/)]
- **Fixing It:** Engineers use special parts called capacitors to supply KVAR right next to the machines, which lowers the strain on the main power lines.

## Displacement reactive power vs. distortion reactive power

**Displacement reactive power** is caused by the time delay between voltage and current waves in linear loads, while **distortion reactive power** is caused by the deformation of wave shapes due to non-linear electronic loads.

In modern power systems, total reactive power is a combination of these two distinct phenomena. [[1](https://www.ytelect.com/blog/reactive-power-kvar-and-reactive-current-amps-in-power-quality_b197), [2](https://www.researchgate.net/publication/375006860_Analysis_of_Reactive_Power_in_Electrical_Networks_Supplying_Non-linear_Fast-Varying_Loads)]

------

### Displacement Reactive Power (\(Q\))

Displacement reactive power is the traditional form of reactive power found in standard alternating current (AC) systems. [[1](https://powerquality.blog/2021/03/10/what-is-reactive-power/)]

- **The Cause:** It occurs when voltage and current waves are perfect sine waves, but they are shifted out of step with each other.
- **The Mechanism:** Inductive loads (like motors and transformers) cause the current wave to lag behind the voltage wave. Capacitive loads cause the current to lead the voltage.
- **The Measurement:** Measured in standard \(Var\) or \(kVar\). It is directly tied to the fundamental frequency (e.g., \(60\text{ Hz}\)).
- **Correction Method:** It can be easily corrected by adding standard **capacitor banks** to realign the waves. [[1](https://www.linkedin.com/pulse/reactive-power-mysterious-critical-system-operation-doering-p-eng), [2](https://www.ziehl-abegg.com/en/glossary/harmonics?srsltid=AfmBOooXfR6MNeSvYVyqlhkanQF53krhON-RO3MCCf9vb4m8lo4kQuV7), [3](https://powerquality.blog/2021/07/16/pq-analysis-power-factor-unbalance-and-harmonics/), [4](https://powermonitors.com/whitepapers/understanding-real-reactive-and-apparent-power/), [5](https://www.ytelect.com/blog/reactive-power-demand-charge-and-compensation-products_b397)]

### Distortion Reactive Power (\(D\))

Distortion reactive power is a modern electrical issue caused by harmonic pollution rather than wave shifting. [[1](https://www.sensorfact.eu/blog/what-is-cosine-phi-and-reactive-power/), [2](https://link.springer.com/article/10.1007/s44291-025-00111-9)]

- **The Cause:** It occurs when non-linear loads change the smooth, round shape of the current wave into a distorted, choppy wave.
- **The Mechanism:** Devices switch power on and off rapidly, drawing current in short pulses. This injects high-frequency frequencies, called harmonics, back into the power system.
- **The Measurement:** Measured in \(VAD\) (Volt-Amperes Distortion) or \(kVar_{dist}\). It exists only when harmonic distortion is present.
- **Common Sources:** Variable speed drives (VFDs), LED lighting, computers, battery chargers, and solar inverters.
- **Correction Method:** Standard capacitors cannot fix this. It requires **harmonic filters** (passive or active) to clean the wave shapes. [[1](https://www.mingchele.com/blog/ups/what-is-the-difference-between-ups-linear-load-and-non-linear-load/), [2](https://electronics360.globalspec.com/article/16418/understanding-harmonic-distortion-created-by-electronics), [3](https://www.scribd.com/document/670130720/Lec-4-Harmonics), [4](https://industrialmonitordirect.com/blogs/knowledgebase/ieee-519-harmonic-distortion-limits-for-industrial-power-systems?srsltid=AfmBOooD5E7XwusZvGjxVAQ4FCx8_OAwDAbC0amM5RBxbqgWU-dfgC0L), [5](https://www.ytelect.com/blog/how-to-calculate-thd-and-pf_b203)]

------

### Key Differences Comparison

| Feature              | Displacement Reactive Power (\(Q\))        | Distortion Reactive Power (\(D\))        |
| -------------------- | ------------------------------------------ | ---------------------------------------- |
| **Wave Shape**       | Perfect sine waves                         | Distorted, non-sinusoidal waves          |
| **Wave Alignment**   | Voltage and current are shifted/misaligned | Voltage and current frequencies mismatch |
| **Primary Culprits** | Large AC motors, transformers, magnets     | Computers, LED lights, VFDs, electronics |
| **Fixing Method**    | **Capacitor banks**                        | **Harmonic filters**                     |

------

### The Mathematical Relationship

In a clean system, total apparent power (\(S\)) depends only on real power (\(P\)) and displacement reactive power (\(Q\)). In a distorted system, distortion power (\(D\)) creates a third dimension: [[1](https://americas.hammondpowersolutions.com/resources/faq/general/why-non-linear-loads-have-low-power-factors-and-why-to-have-a-high-power-factor)]

\(\mathbf{S=}\sqrt{\mathbf{P}^{\mathbf{2}}\mathbf{+Q}^{\mathbf{2}}\mathbf{+D}^{\mathbf{2}}}\)

Where:

- \(S\) = Total Apparent Power (\(kVA\))
- \(P\) = Active/Real Power (\(kW\))
- \(Q\) = Displacement Reactive Power (\(kVar\))
- \(D\) = Distortion Power (\(kVar_{dist}\) or \(VAD\)) [[1](https://eepower.com/technical-articles/total-harmonic-distortion-thd-and-power-factor-calculation/), [2](https://www.vaia.com/en-us/textbooks/physics/basic-electrical-engineering-3-edition/chapter-4/problem-12-voltage-and-current-in-an-ac-circuit-are-given-by/), [3](https://www.linkedin.com/posts/mahmoud-hussien-8922a8229_electrical-power-active-reactive-and-apparent-activity-7277712718311116800-TAS7), [4](https://www.electronics-tutorials.ws/accircuits/reactive-power.html)]

## P, Q, D, and S in a transformer model

In a transformer model, **P, Q, D, and S** represent the different components of electrical power flowing through the transformer under non-linear load conditions. They define how efficiently the transformer transfers energy and how much heat it generates. [[1](https://www.vaia.com/en-us/textbooks/physics/fundamentals-of-electric-circuits-3-edition/chapter-13/problem-71-a-4-mathrmkva-2400-240-mathrmv-mathrmrms-transfor/), [2](https://utbtransformers.com/transformer-efficiency-standards-what-you-need-to-know/)]

Transformers must be sized based on total apparent power (\(S\)) because the total heat generated in the copper windings and steel core depends on the total current, regardless of whether that power is doing useful work. [[1](https://www.mdpi.com/2079-9292/11/15/2398), [2](https://eandisales.com/k-factor-of-transformer/), [3](https://www.weishoelec.com/Blog/how-to-choose-the-right-transformer-size/), [4](https://help.leonardo-energy.org/hc/en-us/articles/202101211-Why-is-the-rating-of-transformers-given-in-kVA-and-not-in-kW)]

------

### Active / Real Power (\(P\))

Active power is the actual useful power transmitted through the transformer to the load to do real work. [[1](https://www.linkedin.com/top-content/engineering/electrical-engineering-power-systems/understanding-kva-ratings-for-transformers-and-generators/), [2](https://fiveable.me/electrical-circuits-systems-ii/unit-6/balanced-unbalanced-three-phase-power-calculations/study-guide/H81a3MS5nCKGguzR)]

- **Definition:** The power converted into useful output, such as mechanical rotation, heat, or light.
- **Unit:** Watts (\(W\)) or Kilowatts (\(kW\)).
- **Transformer Impact:** \(P\) represents the actual energy demand of the customer. It determines the core losses (no-load losses) and contributes to the load losses (copper losses) inside the transformer. [[1](https://www.linkedin.com/posts/balamanickam-n-96ba5b70_pnp-sourcing-and-sinking-explained-in-industrial-activity-7313418674718044160-m0YE), [2](https://capacity4dev.europa.eu/sites/default/files/fiche_5.5_grid_loss.pdf), [3](https://www.scribd.com/document/869966247/Basic-1743734588), [4](https://energypowertransformer.com/transformer-glossaries/), [5](https://www.tycorun.com/blogs/news/calculation-of-transformer-no-load-losses?srsltid=AfmBOop23aCWGsWePwJHkTctZlCIABGOMXE2cjQe-bgN0dThN2-aUZc0)]

### Displacement Reactive Power (\(Q\))

Displacement reactive power is the power absorbed by the transformer itself and inductive loads to maintain magnetic fields.

- **Definition:** The power that continuously sloshes back and forth between the source and the load at the fundamental frequency (\(50\text{ Hz}\) or \(60\text{ Hz}\)) due to the phase shift between voltage and current. [[1](https://www.sciencedirect.com/science/chapter/edited-volume/pii/B9780128233467000128), [2](https://www.mdpi.com/2076-3298/10/10/177)]
- **Unit:** Volt-Amperes Reactive (\(var\)) or Kilovolt-Amperes Reactive (\(kvar\)). [[1](https://www.electronics-tutorials.ws/accircuits/power-triangle.html), [2](https://ieee.li/pdf/introduction_to_power_electronics/chapter_16.pdf), [3](https://www.a-eberle.de/en/knowledge/reactive-power/), [4](https://www.studyforfe.com/blog/how-to-calculate-three-phase-values/)]
- **Transformer Impact:** Transformers *require* \(Q\) to magnetize their steel cores so induction can happen. However, excessive \(Q\) from inductive customer loads (like motors) causes extra current to flow through the transformer windings, creating wasted \(I^{2}R\) heat and lowering voltage stability. [[1](https://vietnamtransformer.com/our-news/power-transformer-ratings/), [2](https://ewh.ieee.org/r3/nashville/events/2019/Improving Efficiency of an Elec Dist System & its Loads Reduces Energy Costs 2019-05-07.pdf)]

### Distortion Power (\(D\))

Distortion power represents the geometric distortion of the power wave caused by harmonic currents flowing through the transformer.

- **Definition:** The non-productive power associated with harmonic frequencies generated by modern non-linear loads (like computers, LED drives, and variable frequency drives). [[1](https://taishantransformer.com/how-to-select-the-right-transformer-capacity/), [2](https://www.gigaenergy.com/blog/electrical-transformers-guide), [3](https://en.wikipedia.org/wiki/Harmonics_(electrical_power))]
- **Unit:** Volt-Amperes Distortion (\(VAD\)) or Distortion \(kvar\).
- **Transformer Impact:** \(D\) is highly dangerous to standard transformers. Harmonic currents cause severe stray load losses and eddy current losses in the windings and structural steel parts. This leads to severe overheating, insulation degradation, and requires the transformer to be **derated** (run below its nameplate capacity) or built as a specialized **K-factor transformer**. [[1](https://ieeexplore.ieee.org/iel7/9961325/9961221/09961438.pdf), [2](https://www.gigaenergy.com/blog/distribution-transformer), [3](https://www.scribd.com/document/317115142/power-and-distribution-transformers-pdf), [4](https://www.sciencedirect.com/science/article/pii/S0378779611003117), [5](https://openurl.ebsco.com/fulltext/gcd:156293550?sid=ebsco:plink:crawler-gcd&id=ebsco:gcd:156293550&crl=f&jrnl=19961073)]

### Total Apparent Power (\(S\))

Total apparent power is the absolute vector sum of all the power components combined. It is the total capacity the transformer must physically support. [[1](https://sparkycalc.com/understanding-the-power-triangle/), [2](https://www.rexpowermagnetics.com/knowledge-hub/how-is-transformer-kva-calculated-a-guide-to-proper-transformer-sizing/), [3](https://esennar.com/blogs/how-to-choose-the-best-power-and-distribution-transformer-for-your-needs.php)]

- **Definition:** The total power delivered from the primary winding to the secondary winding, factoring in the phase shift (\(Q\)) and the harmonic distortion (\(D\)).
- **Unit:** Volt-Amperes (\(VA\)) or Kilovolt-Amperes (\(kVA\)). [[1](https://taishantransformer.com/calculate-load-capacity-of-a-transformer/), [2](https://energypowertransformer.com/transformer-glossaries/), [3](https://medium.com/@electpower/op-5-best-tips-before-buying-single-phase-electric-transformers-in-canada-a26e845c0448)]
- **Transformer Impact:** This is the value printed on the transformer nameplate (e.g., a \(500\text{ kVA}\) transformer). It dictates the physical size of the conductors and the cooling system required. [[1](https://www.facebook.com/Gabriel.Eng.IND/posts/standard-specifications-of-a-power-transformer-number-of-phases-single-or-polyph/1415339057291273/), [2](https://taishantransformer.com/transformer-price-per-kva/)]

------

### The Power Cube Relationship

In modern transformer modeling (defined by IEEE standard 1459), these four values form a 3D orthogonal relationship, often visualized as a power cube rather than a traditional 2D power triangle: [[1](https://ieeexplore.ieee.org/document/9250094/)]

\(\mathbf{S=}\sqrt{\mathbf{P}^{\mathbf{2}}\mathbf{+Q}^{\mathbf{2}}\mathbf{+D}^{\mathbf{2}}}\)

- If the system has **no harmonics**, \(D = 0\), and the formula collapses back to the standard 2D equation: \(S = \sqrt{P^2 + Q^2}\).
- If the system **has harmonics**, \(S\) increases even if \(P\) and \(Q\) stay the same. This means a transformer can overheat from high \(S\) even if the customer's \(kW\) (\(P\)) meter reads low. [[1](https://www.scribd.com/document/464039382/Harmonic), [2](https://www.reddit.com/r/askscience/comments/5bxgul/how_does_the_secondary_current_of_a_transformer/)]

## K-factor

The **K-factor** is a rating applied to a transformer to describe its ability to handle the extra heat generated by harmonic distortion (distortion power, \(D\)) without overheating. [[1](https://www.daelimtransformer.com/k-factor-transformer.html), [2](https://velatron.com/blog/transformer-k-factor/), [3](https://www.larsonelectronics.com/articles/detail/785?srsltid=AfmBOopAOK7_d3l5SldQITefN1NW89QhO26kxNrbO77yKnfjRjLR7Eqq)]

A standard transformer is designed for pure \(60\text{ Hz}\) or \(50\text{ Hz}\) sine waves. When non-linear loads (like computers, variable frequency drives, or LED lighting) pull current in choppy pulses, they create high-frequency harmonic currents. The K-factor quantifies how severe these harmonics are so engineers can choose a transformer that can survive them. [[1](https://www.gigaenergy.com/blog/k-factor-ratings-guide), [2](https://www.nretec.com/blogs/news/k-factor-transformer-guide-ratings-selection-why-non-linear-loads-demand-one), [3](https://www.daelimtransformer.com/k-factor-transformer.html), [4](https://www.rexpowermagnetics.com/knowledge-hub/understanding-the-k-factor-of-transformers-and-harmonics/), [5](https://eandisales.com/uncategorized/k-factor-rated-transformers/)]

------

### The Problem: Why Harmonics Overheat Transformers

When harmonic currents flow through a transformer, they cause two types of waste heat to spike drastically:

1. **Winding Eddy Current Losses (\(P_{EC}\)):** These are circulating currents induced inside the copper or aluminium windings. They increase with the **square of the frequency** (\(f^{2}\)). A 5th harmonic (\(300\text{ Hz}\)) causes 25 times more eddy current heating than the fundamental frequency (\(60\text{ Hz}\)). [[1](https://www.enweielectric.com/blog/k-factor-rated-transformers-handling-harmonics-in-modern-loads), [2](https://eandisales.com/k-factor-of-transformer/)]
2. **Stray Load Losses (\(P_{OS}\)):** These are stray losses in the transformer's steel enclosure, core clamps, and structural bolts, driven by high-frequency magnetic fields.

### Understanding K-Factor Ratings

K-factor is a multiplier. A higher K-factor rating means the transformer has heavier conductors, parallel winding paths, and a specialized core geometry designed to dissipate harmonic heat. [[1](https://kw-engineering.com/vav-box-k-factor-mean-efficient-building-operation/), [2](https://www.ecraftsmen.com/blog/k-rated-transformer-when-you-need-one), [3](https://www.scribd.com/document/296557172/K-Factor-Transformers)]

- **K-1:** Standard transformer. Designed for linear loads only (motors, heaters, incandescent lights). It cannot handle electronic harmonic loads. [[1](https://www.gigaenergy.com/blog/k-factor-ratings-guide), [2](https://www.daelimtransformer.com/guide-to-transformer-harmonics-and-k-factor.html), [3](https://energeks.com/updates/post/transformer-k-factor-harmonics-protection), [4](https://www.nretec.com/blogs/news/k-factor-transformer-guide-ratings-selection-why-non-linear-loads-demand-one)]
- **K-4:** Designed for mild harmonics. Good for standard commercial office buildings with a mix of lighting and some computers. [[1](https://www.daelimtransformer.com/guide-to-transformer-harmonics-and-k-factor.html), [2](https://www.gigaenergy.com/blog/k-factor-ratings-guide), [3](https://www.ecraftsmen.com/blog/k-rated-transformer-when-you-need-one)]
- **K-13:** Designed for heavy harmonics. Used in data centers, server rooms, and school computer labs where electronics dominate the load. [[1](https://energeks.com/updates/post/transformer-k-factor-harmonics-protection), [2](https://www.ryan-transformers.com/news/understanding-the-difference-between-k4-and-k-84979402.html), [3](https://www.maddox.com/resources/articles/k-rated), [4](https://testbook.com/question-answer/k-factor-of-a-transformer-is-the-measure-of-______--67ab479f54dca2709d50fcc9), [5](https://www.diveng.com.au/blog/transformer-k-factor-rating-mitigating-transformer-heat-survival/)]
- **K-20 to K-50:** Designed for extreme harmonics. Used in heavy industrial facilities, hospitals with advanced imaging equipment, and broadcast facilities. [[1](https://www.rexpowermagnetics.com/knowledge-hub/understanding-the-k-factor-of-transformers-and-harmonics/), [2](https://www.idc-online.com/technical_references/pdfs/electrical_engineering/Stability_Factor_K_Factor.pdf), [3](https://www.trystar.com/article/the-importance-of-ki-rated-power-solutions-for-medical-imaging-modalities/)]

------

### The Mathematical Formula

The K-factor is calculated using the Root-Mean-Square (RMS) values of each individual harmonic current, weighted by the square of that harmonic's order number (\(h\)). [[1](https://testbook.com/question-answer/k-factor-of-a-transformer-is-the-measure-of-______--67ab479f54dca2709d50fcc9), [2](https://eandisales.com/uncategorized/k-factor-rated-transformers/), [3](https://www.youtube.com/watch?v=jHuC8CdIXXM)]

Per IEEE C57.110, the formula is:

\(K=\sum _{h=1}^{h_{max}}I_{h}^{2}\cdot h^{2}\)

Where:

- \(h\) = The harmonic order (e.g., \(h=1\) is \(60\text{ Hz}\), \(h=3\) is \(180\text{ Hz}\), \(h=5\) is \(300\text{ Hz}\))
- \(I_{h}\) = The RMS current of the \(h\)-th harmonic, expressed as a **per-unit (percentage)** value of the total RMS current. [[1](https://industrialmonitordirect.com/blogs/knowledgebase/k-factor-transformer-derating-us-vs-european-standards-ieee-c57110?srsltid=AfmBOoqVNd1Hh_Y3VRNc5ES3fjBzCExhDu0PJtIbe4H7K6Go9wLuCn4e), [2](https://testbook.com/question-answer/k-factor-of-a-transformer-is-the-measure-of-______--67ab479f54dca2709d50fcc9), [3](https://www.datsons.com/k-rated-txr.html)]

Because the harmonic order (\(h\)) is squared, high-frequency harmonics rapidly increase the K-factor value, demanding a much tougher transformer. [[1](https://velatron.com/blog/transformer-k-factor/)]

------

### Alternative: Derating Standard Transformers

If a specialized K-rated transformer is too expensive or unavailable, engineers must **derate** a standard K-1 transformer. This means forcing the transformer to run at a lower capacity to prevent it from burning up. [[1](https://www.daelimtransformer.com/guide-to-transformer-harmonics-and-k-factor.html)]

For example, a standard \(100\text{ kVA}\) transformer exposed to a heavy harmonic load (like a K-13 environment) might have to be limited to only \(60\text{ kVA}\) or \(70\text{ kVA}\) of actual load to remain safe. Buying a dedicated K-13 transformer allows the system to utilize the full nameplate capacity safely.