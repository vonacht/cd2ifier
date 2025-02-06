Automatic conversion of CD1 files to CD2.

Basic usage: ```cargo run -- <CD1-source-file> <CD2-target-file>```

where ```CD1-source-file``` is the path to the CD1 file that needs to be converted and ```CD2-target-file``` is the name and path where the result will be written to. 

The script accepts an optional ```-d``` flag to not pretty-print the output. 

In doing the conversion to CD2 the program will take care of the following:

    + Put all fields in the corresponding CD2 top modules (DifficultySetting, Caps, Pools, etc)
    + Remove deprecated fields that are no longer in use or were already useless in CD1 
    + Translate the old pawn stats to the new modules system (Movement, Resistances, etc)
    + Translate StartingNitra, non-existant in CD2, to a mutator
