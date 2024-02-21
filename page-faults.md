# Debug build (using `cargo test --bench io_uring_local`)

## With Box (debug build):

```
~/dev/rust/light-speed-io$ perf stat target/debug/deps/io_uring_local-9905731c0254c179 io_uring_local
Testing load_14_files/io_uring_local
Success

 Performance counter stats for 'target/debug/deps/io_uring_local-9905731c0254c179 io_uring_local':

            102.05 msec task-clock                       #    1.036 CPUs utilized             
                79      context-switches                 #  774.105 /sec                      
                15      cpu-migrations                   #  146.982 /sec                      
             2,948      page-faults                      #   28.887 K/sec                     
       354,009,688      cycles                           #    3.469 GHz                       
       425,560,805      instructions                     #    1.20  insn per cycle            
        89,439,274      branches                         #  876.397 M/sec                     
           617,245      branch-misses                    #    0.69% of all branches           

       0.098460757 seconds time elapsed

       0.036256000 seconds user
       0.063885000 seconds sys
```

## Without Box (debug build):
```
Performance counter stats for 'target/debug/deps/io_uring_local-9905731c0254c179 io_uring_local':

            106.37 msec task-clock                       #    1.032 CPUs utilized             
                87      context-switches                 #  817.902 /sec                      
                24      cpu-migrations                   #  225.628 /sec                      
             2,945      page-faults                      #   27.686 K/sec                     
       367,548,507      cycles                           #    3.455 GHz                       
       424,880,385      instructions                     #    1.16  insn per cycle            
        89,194,976      branches                         #  838.538 M/sec                     
           694,486      branch-misses                    #    0.78% of all branches           

       0.103045399 seconds time elapsed

       0.024904000 seconds user
       0.080349000 seconds sys
```

# Release build (using `cargo test --release --bench io_uring_local`)

## With box (release build)

```
 Performance counter stats for 'target/release/deps/io_uring_local-8248f902089a2c42 io_uring_local':

            100.90 msec task-clock                       #    1.017 CPUs utilized             
                71      context-switches                 #  703.644 /sec                      
                17      cpu-migrations                   #  168.478 /sec                      
             2,784      page-faults                      #   27.591 K/sec                     
       355,223,554      cycles                           #    3.520 GHz                       
       438,183,705      instructions                     #    1.23  insn per cycle            
        93,394,693      branches                         #  925.586 M/sec                     
           560,313      branch-misses                    #    0.60% of all branches           

       0.099214349 seconds time elapsed

       0.020256000 seconds user
       0.077906000 seconds sys
```

## Without box (release build)

```
Performance counter stats for 'target/release/deps/io_uring_local-8248f902089a2c42 io_uring_local':

             74.95 msec task-clock                       #    1.011 CPUs utilized             
                80      context-switches                 #    1.067 K/sec                     
                25      cpu-migrations                   #  333.540 /sec                      
             2,784      page-faults                      #   37.143 K/sec                     
       259,047,709      cycles                           #    3.456 GHz                       
       214,562,221      instructions                     #    0.83  insn per cycle            
        38,850,168      branches                         #  518.323 M/sec                     
           558,571      branch-misses                    #    1.44% of all branches           

       0.074120601 seconds time elapsed

       0.026289000 seconds user
       0.048472000 seconds sys
```

# `cargo bench io_uring_local`

## With box

```
$ cargo bench io_uring_local
     Running benches/io_uring_local.rs (target/release/deps/io_uring_local-d557319dd99ba2ea)
Benchmarking load_14_files/io_uring_local: Collecting 10 samples in estimated 6.2723 s (165 iter
load_14_files/io_uring_local
                        time:   [2.3276 ms 2.5018 ms 2.8618 ms]
                        thrpt:  [1.1944 GiB/s 1.3662 GiB/s 1.4685 GiB/s]
                 change:
                        time:   [-21.977% -3.0364% +20.252%] (p = 0.80 > 0.05)
                        thrpt:  [-16.842% +3.1315% +28.167%]
                        No change in performance detected.

$ perf stat target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local
Testing load_14_files/io_uring_local
Success


 Performance counter stats for 'target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local':

            102.39 msec task-clock                       #    0.982 CPUs utilized             
                76      context-switches                 #  742.256 /sec                      
                35      cpu-migrations                   #  341.828 /sec                      
             2,784      page-faults                      #   27.190 K/sec                     
       361,947,815      cycles                           #    3.535 GHz                       
       392,599,174      instructions                     #    1.08  insn per cycle            
        82,287,964      branches                         #  803.668 M/sec                     
           560,434      branch-misses                    #    0.68% of all branches           

       0.104249216 seconds time elapsed

       0.031260000 seconds user
       0.071116000 seconds sys

$ perf stat target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local --bench --profile-time 2
Benchmarking load_14_files/io_uring_local: Complete (Analysis Disabled)


 Performance counter stats for 'target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local --bench --profile-time 2':

          4,090.53 msec task-clock                       #    2.013 CPUs utilized             
             1,766      context-switches                 #  431.729 /sec                      
               253      cpu-migrations                   #   61.850 /sec                      
            24,317      page-faults                      #    5.945 K/sec                     
    14,499,489,152      cycles                           #    3.545 GHz                       
    21,201,573,619      instructions                     #    1.46  insn per cycle            
     4,659,449,943      branches                         #    1.139 G/sec                     
        11,291,443      branch-misses                    #    0.24% of all branches           

       2.032225610 seconds time elapsed

       0.402250000 seconds user
       3.658198000 seconds sys

$ perf stat target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local --bench
Benchmarking load_14_files/io_uring_local: Collecting 10 samples in estimated 6.9796 s (110 iter
load_14_files/io_uring_local
                        time:   [2.5406 ms 2.7436 ms 3.2384 ms]
                        thrpt:  [1.0554 GiB/s 1.2458 GiB/s 1.3453 GiB/s]
                 change:
                        time:   [-15.861% +1.9243% +23.414%] (p = 0.86 > 0.05)
                        thrpt:  [-18.972% -1.8880% +18.851%]
                        No change in performance detected.


 Performance counter stats for 'target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local --bench':

         16,302.86 msec task-clock                       #    3.287 CPUs utilized             
             8,114      context-switches                 #  497.704 /sec                      
               672      cpu-migrations                   #   41.220 /sec                      
            61,357      page-faults                      #    3.764 K/sec                     
    58,055,980,262      cycles                           #    3.561 GHz                       
    74,460,121,896      instructions                     #    1.28  insn per cycle            
    14,483,128,071      branches                         #  888.379 M/sec                     
       159,545,804      branch-misses                    #    1.10% of all branches           

       4.959796534 seconds time elapsed

       9.736623000 seconds user
       6.529195000 seconds sys

```

## Without Box

```
$ cargo bench io_uring_local
     Running benches/io_uring_local.rs (target/release/deps/io_uring_local-d557319dd99ba2ea)
Benchmarking load_14_files/io_uring_local: Collecting 10 samples in estimated 6.5490 s (110 iter
load_14_files/io_uring_local
                        time:   [2.8493 ms 2.9972 ms 3.3566 ms]
                        thrpt:  [1.0183 GiB/s 1.1404 GiB/s 1.1996 GiB/s]
                 change:
                        time:   [-14.237% +1.5446% +21.356%] (p = 0.87 > 0.05)
                        thrpt:  [-17.598% -1.5212% +16.601%]
                        No change in performance detected.


$ perf stat target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local
Testing load_14_files/io_uring_local
Success


 Performance counter stats for 'target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local':

            104.20 msec task-clock                       #    1.003 CPUs utilized             
                58      context-switches                 #  556.600 /sec                      
                18      cpu-migrations                   #  172.738 /sec                      
             2,787      page-faults                      #   26.746 K/sec                     
       373,282,534      cycles                           #    3.582 GHz                       
       375,320,855      instructions                     #    1.01  insn per cycle            
        78,321,805      branches                         #  751.619 M/sec                     
           555,646      branch-misses                    #    0.71% of all branches           

       0.103932787 seconds time elapsed

       0.034473000 seconds user
       0.069692000 seconds sys

$ perf stat target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local --bench --profile-time 2
Benchmarking load_14_files/io_uring_local: Complete (Analysis Disabled)


 Performance counter stats for 'target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local --bench --profile-time 2':

          3,749.19 msec task-clock                       #    1.935 CPUs utilized             
             1,532      context-switches                 #  408.622 /sec                      
               206      cpu-migrations                   #   54.945 /sec                      
            56,134      page-faults                      #   14.972 K/sec                     
    13,269,838,131      cycles                           #    3.539 GHz                       
    19,187,529,360      instructions                     #    1.45  insn per cycle            
     4,193,027,395      branches                         #    1.118 G/sec                     
        10,583,304      branch-misses                    #    0.25% of all branches           

       1.937626603 seconds time elapsed

       0.378301000 seconds user
       3.342780000 seconds sys

$ perf stat target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local --bench
Benchmarking load_14_files/io_uring_local: Collecting 10 samples in estimated 7.0158 s (110 iter
load_14_files/io_uring_local
                        time:   [3.1616 ms 3.4601 ms 3.8322 ms]
                        thrpt:  [913.31 MiB/s 1011.5 MiB/s 1.0811 GiB/s]
                 change:
                        time:   [-5.8614% +9.3418% +26.838%] (p = 0.26 > 0.05)
                        thrpt:  [-21.159% -8.5436% +6.2263%]
                        No change in performance detected.


 Performance counter stats for 'target/release/deps/io_uring_local-d557319dd99ba2ea io_uring_local --bench':

         17,448.78 msec task-clock                       #    3.402 CPUs utilized             
             6,796      context-switches                 #  389.483 /sec                      
               560      cpu-migrations                   #   32.094 /sec                      
           139,743      page-faults                      #    8.009 K/sec                     
    58,212,193,060      cycles                           #    3.336 GHz                       
    74,080,876,538      instructions                     #    1.27  insn per cycle            
    14,308,349,752      branches                         #  820.020 M/sec                     
       163,963,630      branch-misses                    #    1.15% of all branches           

       5.128500679 seconds time elapsed

      10.718327000 seconds user
       6.660812000 seconds sys
```