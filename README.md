Automatic conversion of CD1 files to CD2. To run the script you will need to have Rust installed in your system. 

Basic usage: ```cargo run -- <CD1-source-file> [CD2-target-file]```

where ```CD1-source-file``` is the path to the CD1 file that needs to be converted and ```CD2-target-file``` is the name and path where the result will be written to.
The target file path is optional, and if not specified, the script will save the result in the same directory where it is executed with the name of the original file
and ".cd2" appended before the extension, if applicable. 

The script accepts an optional ```-d``` flag to not pretty-print the output, resulting in a JSON in compact form. 

In doing the conversion to CD2 the program will take care of the following:

+ Put all fields in the corresponding CD2 top modules (DifficultySetting, Caps, Pools, etc)
+ Remove deprecated fields that are no longer in use or were already useless in CD1 
+ Translate the old pawn stats to the new modules system (Movement, Resistances, etc)
+ Translate StartingNitra, non-existant in CD2, to a mutator

## Limitations
The script accepts multiline descriptions as commonly found in difficulty files, but not multiline names. If that is your case, the multilines in the name 
will have to be removed manually before proceeding with the conversion. 
