---
emem: pointer.v1
source: https://huggingface.co/openai-community/gpt2/resolve/main/model.safetensors
bytes: 548105171
etag: "63bed80836ee0758c8fd4f8975d59bb0b864263ee2753547c358e8a37cde8758"
kind: model weights (safetensors)
chunks: 107 of 161 hashed
root: iwrj5zn26agrtpusdy2qvskarvqc27toftqfctju5zswh5r3f4kq
hash: blake3-256 of each chunk's bytes
order: defaults, then by blake3(label without type or shape)
---

# model.safetensors

> model weights (safetensors) at huggingface.co, 548.1 MB. The data stays there; this note is its address and its proofs. Read any chunk from the source by URL and byte range, then check its BLAKE3 hash below. The root is a Merkle tree over (url, offset, length, hash) of every row, in order.

- 160 tensors, 137,022,720 parameters, F32
- the header pins the layout of every tensor; each hashed tensor also carries its mean, spread and range
- metadata: {"format":"pt"}
- 107 of 161 chunks hashed; more can be hashed later, in a fixed order, without re-reading these

## Chunks

| what | url (· is the source) | offset | length | blake3 | stats |
|---|---|---|---|---|---|
| header: every tensor's name, dtype, shape and byte range | · | 0 | 14291 | vtqobol7vtartdjlje4rsv4aqxogghursbpy3lt3n6wvqnzzzhqq |  |
| tensor h.3.attn.c_proj.bias F32[768] | · | 14291 | 3072 | 34yljtchrttv3cekc2i4tv62d6sh4bx3476g26hc5cuuvkqdfgtq | mean -0.001561 · sd 0.1078 · min -1.027 · max 0.5182 |
| tensor h.2.mlp.c_fc.bias F32[3072] | · | 17363 | 12288 | asmhupwwrzzjvkkz7hzdlsv4orij54qwi4w754xqmji5w2kusava | mean -0.09282 · sd 0.1067 · min -0.6598 · max 1.731 |
| tensor h.3.mlp.c_proj.bias F32[768] | · | 29651 | 3072 | mr44y3yr5nuwxiea6jvtylro4cpqwxc4qeex2zld4bpr6og5oysa | mean 0.002102 · sd 0.115 · min -0.6661 · max 1.864 |
| tensor h.5.ln_1.weight F32[768] | · | 2392019 | 3072 | keyma32ztunc34sa3t35p37h7vjn7ideyydujlsdgagew47vqmla | mean 0.3731 · sd 0.045 · min 0.08638 · max 0.7687 |
| tensor h.1.attn.c_attn.weight F32[768,2304] | · | 2395091 | 7077888 | lvzrxfkvh42cmmwk646uap3qdamxmgzyg6crr2lhhldmahu6o5qa | mean 0.00002809 · sd 0.1401 · min -1.077 · max 1.238 |
| tensor ln_f.weight F32[768] | · | 9472979 | 3072 | mgw33pp4uys3oqe4gu7rrwpnrxvp343nqvodzjcqdlvlbny3cqkq | mean 1.508 · sd 1.39 · min 0.004427 · max 17.42 |
| tensor h.9.attn.c_attn.bias F32[2304] | · | 9476051 | 9216 | h4cxspwssf3zyziwn7e2btfg6usaaiqquhkx3tuk4ftg7wyrghpa | mean 0.0002855 · sd 0.1405 · min -1.042 · max 0.8039 |
| tensor h.0.attn.c_proj.bias F32[768] | · | 9485267 | 3072 | gjfjq7zf2yoo5epxrzevykfdszhtimq7omnewmpulbbmqyklv6iq | mean -0.00691 · sd 0.2588 · min -2.684 · max 2.03 |
| tensor h.3.attn.c_attn.bias F32[2304] | · | 18925523 | 9216 | yjiipcsbsllbzpxlwk2evvosehexety6w5m2qhta4l7jkdmeeyga | mean -0.0008886 · sd 0.1417 · min -0.6688 · max 0.7118 |
| tensor h.3.mlp.c_fc.bias F32[3072] | · | 18934739 | 12288 | txwq5hgdscjdalsppbjjrktwve3cs7exbro7j4ll6vy36bcbxb4a | mean -0.09253 · sd 0.08565 · min -1.226 · max 0.5168 |
| tensor h.5.attn.c_attn.bias F32[2304] | · | 18947027 | 9216 | bemu3nozlrnnh2iis7on6les2fs5vh64jpldvlfea2guifkyvr7a | mean 0.001046 · sd 0.09922 · min -0.5594 · max 0.4744 |
| tensor h.0.mlp.c_fc.bias F32[3072] | · | 18956243 | 12288 | 5crv2fttaj23n34chgpcle3oh7bmyywxf6ftvpo7lyo47o46scdq | mean -0.09316 · sd 0.1323 · min -0.7462 · max 0.3323 |
| tensor h.5.mlp.c_proj.bias F32[768] | · | 21327827 | 3072 | 3pxd5aj4woraxx7i47f4uz6lsh6xfecj4gdgxx6ilplyud3yaleq | mean 0.0009533 · sd 0.1064 · min -0.712 · max 1.253 |
| tensor h.11.attn.c_proj.bias F32[768] | · | 21330899 | 3072 | jhun4f7erd6i3syvap23cowqszvaqon5rit6ngajjyfsmjfvyfoa | mean -0.02151 · sd 0.4686 · min -5.373 · max 3.636 |
| tensor h.2.attn.c_attn.bias F32[2304] | · | 28411859 | 9216 | 6xuyc5tgrohl3dpaznttylthocnetfponqdk2ijhiccayztcjnua | mean -0.003877 · sd 0.1648 · min -1.463 · max 1.177 |
| tensor h.7.attn.c_proj.bias F32[768] | · | 37858259 | 3072 | oh5oq66pdbz5emfqq5dwb2eqb57i2cynkb7domjunolqfsb2psaq | mean -0.0001336 · sd 0.145 · min -0.4598 · max 0.5181 |
| tensor h.9.mlp.c_fc.bias F32[3072] | · | 37861331 | 12288 | gzoka4tewhg564od63twp43u6dbro6hoyqurnqhserjlnjbo4d4q | mean -0.08367 · sd 0.09235 · min -0.4776 · max 0.6103 |
| tensor h.5.ln_2.bias F32[768] | · | 40232915 | 3072 | ws6l55clyen5gmbbilsnbmbnxupsnohme7wnvsg5vewsiy5s6pla | mean 0.008152 · sd 0.03278 · min -0.3045 · max 0.3178 |
| tensor h.5.ln_1.bias F32[768] | · | 40235987 | 3072 | a36mx6nskdayfeuqqjq57dtzwzw4zq7iq5nc77vadosehcy4q7ka | mean 0.01188 · sd 0.04903 · min -0.4506 · max 1.079 |
| tensor h.11.ln_2.weight F32[768] | · | 40239059 | 3072 | 6q6kamz5p6szv3ufynvionf5s6kze6iautiwfvddsj53tc4s2cjq | mean 0.5041 · sd 0.08995 · min 0.02785 · max 1.233 |
| tensor h.9.ln_2.weight F32[768] | · | 53873619 | 3072 | uyna7u3rwm2so57tdpqnfxu5whillhn7x6bcfw5j6bvafhwlnroq | mean 0.265 · sd 0.04149 · min 0.01774 · max 0.9476 |
| tensor h.4.ln_2.bias F32[768] | · | 53876691 | 3072 | medlcduix6cssodelw5pchzwtckzkopuoyqcivipyxkm2qpyabaa | mean 0.0009597 · sd 0.02687 · min -0.1421 · max 0.1429 |
| tensor h.7.mlp.c_proj.bias F32[768] | · | 53879763 | 3072 | s5stn43xhwdgrdn2ij34funt2nlbnamzes4e3geq2edil2mmykca | mean 0.001193 · sd 0.1287 · min -0.7256 · max 1.177 |
| tensor h.11.ln_1.bias F32[768] | · | 53882835 | 3072 | 42crjse6lveqo377ldnnmercdr7wtbzpr6pyvhwa2tyddqxvmp4q | mean 0.02328 · sd 0.05671 · min -0.3304 · max 1.004 |
| tensor h.1.ln_2.weight F32[768] | · | 53885907 | 3072 | ejotuuj2qfv53wnfzmya2bj4bshqwdfo3cpckvr242tawg5vdy4a | mean 0.2427 · sd 0.03162 · min 0.05604 · max 0.4523 |
| tensor h.4.mlp.c_fc.bias F32[3072] | · | 53888979 | 12288 | w4gax5xvbmb4xvjqdpay5o56kbtc4v4xaafurazklx66sh5kvi4q | mean -0.08613 · sd 0.09319 · min -0.453 · max 0.7417 |
| tensor h.0.ln_2.weight F32[768] | · | 67532755 | 3072 | oe72ydsbyzpms7oldmxkcqoomtvt4qgihxn6eunbng3bmygqr7wa | mean 0.8678 · sd 0.4846 · min 0.04529 · max 1.511 |
| tensor h.4.attn.bias F32[1,1,1024,1024] | · | 67535827 | 4194304 | 3spctcw6gzq3pg4syx4kigimmw5vvbrrgrkrokdaxumotpu4oyyq | mean 0.5005 · sd 0.5 · min 0 · max 1 |
| tensor h.2.attn.c_proj.bias F32[768] | · | 85361619 | 3072 | in5nzoavduiobtg7edd5xn3bja2idjjxeq2qkhpnqapmmkshc3cq | mean 0.003376 · sd 0.145 · min -0.4775 · max 0.5149 |
| tensor h.2.ln_1.weight F32[768] | · | 85364691 | 3072 | ssfeymk3zhpziywl2pnabzh35kjpmibiq5j4amgqtx5lvxdfrxcq | mean 0.2408 · sd 0.07522 · min 0.04599 · max 0.9443 |
| tensor h.11.attn.c_attn.bias F32[2304] | · | 85367763 | 9216 | vhkhqw7bhaia36xiyvfcss27zkbtkp6bdpmttsdnyzz2ut64jy5a | mean 0.0007319 · sd 0.121 · min -0.8046 · max 0.5992 |
| tensor ln_f.bias F32[768] | · | 85376979 | 3072 | hz2a534vwnbyjmvfqtv4nbmsex7glaq5nvxqbk2jjihonyqqtoiq | mean -0.003138 · sd 0.4194 · min -4.192 · max 7.368 |
| tensor h.0.attn.c_attn.bias F32[2304] | · | 85380051 | 9216 | lre7b2s77ekjpgjszu7kkjtybndl5ydjk6x4zeavi2vedyunp3ea | mean -0.0007073 · sd 0.2259 · min -1.337 · max 1.175 |
| tensor h.5.mlp.c_fc.bias F32[3072] | · | 85389267 | 12288 | xwdvcxquwu6f4pwuqwqzprxdyr7af63mfms6fw6wd5j6hjv3p7rq | mean -0.08503 · sd 0.08899 · min -0.4335 · max 0.6701 |
| tensor h.7.ln_2.weight F32[768] | · | 85401555 | 3072 | 7v7lmwham5macar6wdwnq6gbcup3l3pirtnfum4cvebk4syvshpa | mean 0.256 · sd 0.04702 · min 0.01492 · max 1.293 |
| tensor h.1.mlp.c_proj.bias F32[768] | · | 94841811 | 3072 | qn3o4fjoau4q6w4me4z2su3niusdmsp5dxr6elq7c63smn42pocq | mean 0.0002506 · sd 0.1004 · min -0.6991 · max 1.594 |
| tensor h.1.ln_2.bias F32[768] | · | 94844883 | 3072 | bdgszexevhutzphi3i2ufl3c24p2jehwn2frmfyb3zofq36llhqa | mean -0.004074 · sd 0.03917 · min -0.5866 · max 0.4565 |
| tensor h.4.mlp.c_proj.bias F32[768] | · | 99042259 | 3072 | x6ypnacpveptajyll5egzdpkppnuy6f3357cp2u4s437zasppgaa | mean 0.001606 · sd 0.1368 · min -0.6683 · max 1.569 |
| tensor h.8.ln_2.bias F32[768] | · | 132075475 | 3072 | mvvnurygoklpmmgloiwkrt5cychlywd3z6icw2d6uxgeukl6xawa | mean 0.0003844 · sd 0.04934 · min -0.695 · max 0.4522 |
| tensor h.11.mlp.c_fc.bias F32[3072] | · | 132078547 | 12288 | mvpl4zd57vh4x2wi3zfldsv3wz3q4uqmueaw246vbppxq4j2qhcq | mean -0.06411 · sd 0.09303 · min -1.208 · max 0.5148 |
| tensor h.7.mlp.c_fc.bias F32[3072] | · | 139168723 | 12288 | n2jwppirqw4gwmb4a5egue5laoilvzlqij6egf4rbfdfshfvssca | mean -0.08847 · sd 0.09063 · min -0.7277 · max 0.832 |
| tensor h.6.mlp.c_proj.bias F32[768] | · | 150977491 | 3072 | zc736scadyumed5bm2kvnzik24pegzyrumbvghvrhqoseravllqa | mean 0.001563 · sd 0.121 · min -0.6757 · max 1.046 |
| tensor h.8.attn.c_proj.bias F32[768] | · | 150980563 | 3072 | 5tf43hdipegqjdclry3el3qxz6jnz6reuzvdvczo4ak6o6xtetdq | mean 0.001086 · sd 0.14 · min -0.4676 · max 1.157 |
| tensor h.10.mlp.c_proj.bias F32[768] | · | 150983635 | 3072 | lpb64ahjeh633owxbdkb3dm7pa6s2x3gti6melsfen7ayd6knueq | mean 0.001659 · sd 0.1939 · min -1.077 · max 1.34 |
| tensor h.4.ln_1.bias F32[768] | · | 150986707 | 3072 | e4ucucum4hbm4rtl7sk3uknlny2zv4jlg5zlkzaxewa6otcowodq | mean 0.007916 · sd 0.06745 · min -0.5446 · max 1.551 |
| tensor h.4.attn.c_attn.bias F32[2304] | · | 150989779 | 9216 | xhbeaeytgzkrvpcad4onov67mmvipxkjnesr3czu5w2lcaqgonsq | mean 0.005201 · sd 0.2395 · min -2.612 · max 2.746 |
| tensor h.1.attn.c_proj.bias F32[768] | · | 160436179 | 3072 | hbkrto6ojuzk6awr6lsek5brnx64czdcremzza3hixyglurng6la | mean -0.001073 · sd 0.1047 · min -0.5346 · max 1.251 |
| tensor h.11.mlp.c_proj.weight F32[3072,768] | · | 160439251 | 9437184 | i5uwzmdcbqzr54qpn4k5o3tadkllailg6woqrphyxa2ugnwpwnca | mean -0.0004353 · sd 0.1982 · min -9.212 · max 9.146 |
| tensor h.6.ln_2.weight F32[768] | · | 169876435 | 3072 | juqiddteed622bceuhjy7judfxhvymlf5okbggcdpmnxikjodrmq | mean 0.2595 · sd 0.04737 · min 0.04236 · max 1.34 |
| tensor h.9.ln_2.bias F32[768] | · | 169879507 | 3072 | hvg2lkxnaikr42brjgzor5v2ja2qbjmob32h4r7hrgqi7yg4tubq | mean 0.006401 · sd 0.04568 · min -0.561 · max 0.5137 |
| tensor h.6.ln_2.bias F32[768] | · | 169882579 | 3072 | 4env54l2oh6ej4fk5pngxkioejxa67u2t2h2uwxkzecxtml7ax6q | mean 0.004331 · sd 0.0338 · min -0.2726 · max 0.4521 |
| tensor h.5.attn.c_proj.bias F32[768] | · | 169885651 | 3072 | tjqfv3wn4pyen7np3he3csnbd4botjo56qtvc6koprcerdmsunxa | mean -0.001168 · sd 0.1107 · min -0.703 · max 0.5939 |
| tensor h.7.ln_1.bias F32[768] | · | 169888723 | 3072 | dmatp7rqmsehzalrlch4sslzfn4xxzoqntozbgrzxek6plkkdnaa | mean 0.01434 · sd 0.05911 · min -0.7903 · max 1.196 |
| tensor h.1.ln_1.weight F32[768] | · | 172251091 | 3072 | lm54x566t3ozib2tdoolpm2e7banxtvfl6yft3ggk3rjgjeh5naa | mean 0.2228 · sd 0.05127 · min 0.07251 · max 0.6553 |
| tensor h.7.ln_2.bias F32[768] | · | 172254163 | 3072 | vlt2dfmp6mgc4rr7c2ey5q5bqg2wdrom7ysfzxsnrminml7feqca | mean 0.009184 · sd 0.04568 · min -0.4666 · max 0.6272 |
| tensor h.2.ln_2.weight F32[768] | · | 176451539 | 3072 | 3qfzbyewowxaqen724emex3y66wjufhnhrqtzv7tkcgtly7k536a | mean 0.2926 · sd 0.04538 · min 0.04211 · max 0.7296 |
| tensor h.10.attn.c_attn.bias F32[2304] | · | 176454611 | 9216 | vue4qdgymzwjqolvoucilnkf6ywijd3kyegthsjmmmzmldw3lmma | mean 0.001611 · sd 0.1452 · min -0.9162 · max 0.771 |
| tensor h.10.attn.c_proj.bias F32[768] | · | 181182419 | 3072 | 72lorrth37od6at35ovyibe4twu3s3szah3vep3thmtquqkruw2q | mean 0.002024 · sd 0.2321 · min -2.988 · max 3.849 |
| tensor h.8.mlp.c_fc.bias F32[3072] | · | 181185491 | 12288 | 2ptqpr2rr5vxhvc27libz2apvlfp32vzccgh6vcy63my2j7vshjq | mean -0.08506 · sd 0.09353 · min -0.5317 · max 0.9932 |
| tensor h.9.ln_1.bias F32[768] | · | 185916371 | 3072 | 45yqamfu2vbws5ywg22mrta6zom5zlq5sac7z2fhcj5ecd2tdn6a | mean 0.0159 · sd 0.06383 · min -0.9785 · max 1.264 |
| tensor h.4.mlp.c_fc.weight F32[768,3072] | · | 185919443 | 9437184 | ukp5rpmbvvvyctto24tazagzpl3wkiljmsp3xyi6kf7a5ir7ncva | mean -0.003264 · sd 0.1297 · min -2.161 · max 2.02 |
| tensor h.3.ln_2.bias F32[768] | · | 202434515 | 3072 | 2biqw6tinwwnjajqtvjsnvzemshr3ekd5brsufqx7bixx3nizuqa | mean 0.009965 · sd 0.04355 · min -0.4375 · max 0.4349 |
| tensor h.5.ln_2.weight F32[768] | · | 202437587 | 3072 | q7v4nu3cw6cgckf7rpafipchbvtrc377zm5tlugnujaorzayxf6a | mean 0.279 · sd 0.05144 · min 0.04285 · max 1.418 |
| tensor h.8.mlp.c_proj.bias F32[768] | · | 206634963 | 3072 | qljxvvjdrax4nnb5e2tfr4hlz5noxmobebxwj4dhagpu2p7ozw3a | mean 0.001156 · sd 0.1273 · min -0.8268 · max 1.22 |
| tensor h.4.ln_1.weight F32[768] | · | 206638035 | 3072 | hwbrccrcvz6fdmxamy3bigz3ydb7csbayu6cg3tfce4hkybznpoq | mean 0.3193 · sd 0.04736 · min 0.05798 · max 0.6705 |
| tensor h.0.ln_2.bias F32[768] | · | 206641107 | 3072 | tt4jlb47l4j6ldsx5vch6f5442q77kta6wfvbq7iq7vqfjbmz7uq | mean 0.009204 · sd 0.07005 · min -0.6648 · max 0.7394 |
| tensor h.10.ln_2.weight F32[768] | · | 213722067 | 3072 | 5ty2qx6itvodf7m3aaxds2qjlrvajwitv3yl6jzfv5wiecm45fja | mean 0.2897 · sd 0.05114 · min 0.02009 · max 1.095 |
| tensor h.1.ln_1.bias F32[768] | · | 213725139 | 3072 | ikfsglggj7cz7krfrzpf7iwsa3f4sd3bvf7uff2lsuiem5ors3la | mean -0.005023 · sd 0.0524 · min -0.6645 · max 0.5332 |
| tensor h.2.ln_2.bias F32[768] | · | 223165395 | 3072 | xdenw5juxjqz5n6agv6xrczj35h4sc5jxrhts5k6ulfvoucunpoq | mean 0.006348 · sd 0.0439 · min -0.647 · max 0.37 |
| tensor h.10.ln_1.weight F32[768] | · | 223168467 | 3072 | lm4fqgfhxjkyc6ghztjvsxvn2vf4ia5mec5ofq5prod36jibk2dq | mean 0.3782 · sd 0.05566 · min 0.07506 · max 0.9214 |
| tensor h.6.mlp.c_fc.bias F32[3072] | · | 232608723 | 12288 | ju3g7aj3ol5xt5dspftqjhk42kddr3zgpmadlkngmkjcrcazadcq | mean -0.0857 · sd 0.09055 · min -0.431 · max 0.6824 |
| tensor h.7.attn.c_attn.bias F32[2304] | · | 242058195 | 9216 | 774z3mkjatsbi754bndjmnqroiksq73fc6zouctjc72wl24xp4ba | mean -0.004283 · sd 0.1381 · min -0.7567 · max 0.7275 |
| tensor h.8.ln_1.bias F32[768] | · | 242067411 | 3072 | hu4fotdondk55tm6hpiwpl6xibikovyfdwpzgnyoalnvq3xwldcq | mean 0.01325 · sd 0.06866 · min -0.9212 · max 1.454 |
| tensor h.1.attn.c_attn.bias F32[2304] | · | 242070483 | 9216 | c7d5bphflqcykabfht75idrjjp4qkdxmkm6isxuwghvos2nbq2rq | mean 0.0008003 · sd 0.211 · min -1.815 · max 1.939 |
| tensor h.6.attn.c_attn.bias F32[2304] | · | 242079699 | 9216 | ei4xyschutr4imd5hs2y4csvs5wnb3ngbrngakza6yrc7jizxbjq | mean 0.001821 · sd 0.123 · min -0.8114 · max 0.7243 |
| tensor h.3.ln_1.bias F32[768] | · | 242088915 | 3072 | k3aqmxcydjk76ljpq2bkfgiimndy677racje3gs3gz7m4z75jviq | mean 0.005448 · sd 0.07011 · min -0.4335 · max 1.737 |
| tensor h.0.ln_1.bias F32[768] | · | 242091987 | 3072 | fmuefcqzdjkrddgbwrim76c4nqkfj2ehttou3pntpziz4kl7jmrq | mean -0.006593 · sd 0.03578 · min -0.2589 · max 0.2019 |
| tensor h.4.ln_2.weight F32[768] | · | 242095059 | 3072 | k2q2n6fl3vsoc742f63n7fdzosx5jbz6eqp5dycdqufzturwrpiq | mean 0.2726 · sd 0.04387 · min 0.07007 · max 1.132 |
| tensor h.8.attn.c_attn.bias F32[2304] | · | 251535315 | 9216 | u6xqomcefsr3wniy3iemr6mjiql4d2xqkhfiqgp6fgkantlk4gua | mean -0.00553 · sd 0.1314 · min -0.869 · max 0.837 |
| tensor h.8.ln_1.weight F32[768] | · | 251544531 | 3072 | xw5xonnt6nuchxy3iqjqh7f3efenmlcqv4p2yaxb5365olmfkqba | mean 0.3352 · sd 0.04451 · min 0.06567 · max 0.9248 |
| tensor h.8.ln_2.weight F32[768] | · | 263344083 | 3072 | 4avoxbpq3dhm6kku2vzf4h4hmbufodiock6r4geh3oyet6leayqa | mean 0.2567 · sd 0.04125 · min 0.01805 · max 1.072 |
| tensor h.0.mlp.c_proj.bias F32[768] | · | 263347155 | 3072 | vxoguakqmimob56t4re4ubtbqmbx44zzr6zvwmfabffe3qx3mqfq | mean -0.0004231 · sd 0.1016 · min -1.029 · max 1.479 |
| tensor h.6.attn.c_proj.bias F32[768] | · | 263350227 | 3072 | akgzb5tagfha7krdeciofimzefzzyweqmra4lkug52fjnpjadpaq | mean -0.0004364 · sd 0.106 · min -0.3124 · max 0.396 |
| tensor h.11.ln_2.bias F32[768] | · | 270693331 | 3072 | m7l3kreexuo4pogsgw3nhewkuskhdwmx4dhgkqmh4avbdjyd4cxq | mean 0.009193 · sd 0.03914 · min -0.2093 · max 0.4247 |
| tensor h.0.attn.c_attn.weight F32[768,2304] | · | 270696403 | 7077888 | jvizte7kcbikvm7nrswctl7evrcvpq63dmqs5rzrjfojz6mqhx7q | mean 0.00005338 · sd 0.1996 · min -2.844 · max 2.796 |
| tensor h.1.mlp.c_fc.bias F32[3072] | · | 277774291 | 12288 | hnsd3d7553lcbz3rgcyhlo3usbxepbnnvxdphq6mffvm4pfs27nq | mean -0.0722 · sd 0.0949 · min -0.6563 · max 0.2651 |
| tensor h.9.ln_1.weight F32[768] | · | 441613267 | 3072 | m7hcrt2he553rqvgblslk6s5dek3te224sfyppdalw2c6r6eqpuq | mean 0.3576 · sd 0.04691 · min 0.06961 · max 0.945 |
| tensor h.11.ln_1.weight F32[768] | · | 441616339 | 3072 | wsbudxjl4jotcggvsu5g3e2ntdcy7hhfvnq4w2vdxtjjxg7ojira | mean 0.4787 · sd 0.06512 · min 0.1061 · max 0.9577 |
| tensor h.7.mlp.c_fc.weight F32[768,3072] | · | 441619411 | 9437184 | ibv3dvjdxkmicq6hofzz57tfi5vflsdhb7xm42slzrf2suhvqjeq | mean -0.003523 · sd 0.1264 · min -0.8502 · max 1.232 |
| tensor h.0.ln_1.weight F32[768] | · | 451056595 | 3072 | mf7b4ejkpj3qtjmo6dzvvjy4nub7xajrws676jwoj3uhoja5syva | mean 0.1804 · sd 0.04129 · min 0.04186 · max 0.2527 |
| tensor h.6.ln_1.weight F32[768] | · | 451059667 | 3072 | ba7isnxa6liehfmq4hpfsj7e3amm2yf3xdp7256gnliip4346l4q | mean 0.3456 · sd 0.04414 · min 0.06372 · max 0.7796 |
| tensor h.3.ln_1.weight F32[768] | · | 451062739 | 3072 | jt5hhp34rj2kq3wbs76i2hqhqsm2grhyydyp5eku2sarkrfsysga | mean 0.3011 · sd 0.05348 · min 0.05652 · max 0.7677 |
| tensor h.9.mlp.c_proj.bias F32[768] | · | 451065811 | 3072 | 2nwfk6dhju2lkl2vpbok2bb6aqzxbx3iiogrokrj7uun4srlsiea | mean 0.0007137 · sd 0.1591 · min -1.283 · max 1.49 |
| tensor h.2.ln_1.bias F32[768] | · | 451068883 | 3072 | h5qpk42rh2bo7jdtvyvur2kjtrlfwy2ihcjfkpiqex5y56i4c2na | mean -0.0003601 · sd 0.07051 · min -0.5625 · max 1.094 |
| tensor h.2.mlp.c_proj.bias F32[768] | · | 451071955 | 3072 | oev364qahqrlzfrtycz3jbawbbdgoi34xiqzbilojy5fsd2czova | mean 0.002819 · sd 0.1124 · min -0.4528 · max 1.568 |
| tensor h.6.ln_1.bias F32[768] | · | 460512211 | 3072 | ztg4cgy4dhjz6c3n47arhusdatkza26w3c26f5qoouwvrsppr7eq | mean 0.01182 · sd 0.06616 · min -0.6354 · max 1.522 |
| tensor h.1.attn.c_proj.weight F32[768,768] | · | 513730515 | 2359296 | hxxznw4re3rv3fwou7jum5xbsy6ol3n6l6enq76o3dn2zqgnpfna | mean -0.00008276 · sd 0.1019 · min -4.726 · max 3.986 |
| tensor h.7.ln_1.weight F32[768] | · | 516089811 | 3072 | o2mjk2a32eb3umnabrlwqm5yv2tubrm5mprchwqhbj7zzb4givta | mean 0.3566 · sd 0.04376 · min 0.0746 · max 0.8183 |
| tensor h.10.ln_2.bias F32[768] | · | 516092883 | 3072 | ihtplnwkwk6d5qvb57mwbl26euld6d5ukwucjc4c3fevpoowqmwa | mean 0.02116 · sd 0.04485 · min -0.6005 · max 0.6914 |
| tensor h.10.mlp.c_fc.bias F32[3072] | · | 520290259 | 12288 | fgunu7oxy7sxb3hic7smlghh6nxzrgifcbnntm5wpd7fcobbsy7q | mean -0.07652 · sd 0.09121 · min -1.073 · max 0.7247 |
| tensor h.10.ln_1.bias F32[768] | · | 520302547 | 3072 | 52law3sp62unr4mvbjzdhywl64m442i4gonnozhp6ryueo7yh54a | mean 0.01861 · sd 0.05503 · min -0.6528 · max 1.09 |
| tensor h.4.attn.c_proj.bias F32[768] | · | 520305619 | 3072 | emahoqf5dvl6aoyzba67xktsyvqbboljzkf4sgmaufmn2hsazbsq | mean -0.0009013 · sd 0.1004 · min -0.643 · max 0.5158 |
| tensor h.11.mlp.c_proj.bias F32[768] | · | 520308691 | 3072 | wuvmxj5qvdhyxffcevxqacdhmbi5z4bklqelxfox5h4r2hlje5eq | mean 0.0009716 · sd 0.1082 · min -0.3835 · max 0.4373 |
| tensor h.10.attn.bias F32[1,1,1024,1024] | · | 520311763 | 4194304 | 3spctcw6gzq3pg4syx4kigimmw5vvbrrgrkrokdaxumotpu4oyyq | mean 0.5005 · sd 0.5 · min 0 · max 1 |
| tensor h.3.ln_2.weight F32[768] | · | 524506067 | 3072 | yyxqihrxibxga6uhz2x5wjqsr3puwtauoefiitja6jodife6a5qa | mean 0.3065 · sd 0.05262 · min -0.0002557 · max 1.16 |
| tensor h.9.attn.c_proj.bias F32[768] | · | 531587027 | 3072 | gx2qza2niedxi4b6tt2gg7bygzr6lggin63hzy536gwxf3fiitta | mean 0.002134 · sd 0.2094 · min -0.9575 · max 1.896 |
