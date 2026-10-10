// Authenticated inputs: docs/formats/fixed-control.md, Initial registry inputs.
use super::initial_registry::Pin;

// Constructor defaults: docs/formats/fixed-control.md, Conditional plain text.
pub const PLACE_DEFAULT_FIELDS: [usize; 4] = [0, 1, 4, 6];
pub const ITEM_DEFAULT_FIELDS: [usize; 4] = [4, 5, 11, 19];

pub const COMMAND_CONFIGURATION: [Pin<'static>; 2] = [
    Pin {
        id: 0x01030000,
        size: 7044,
        sha256: "860023878f9ad1e98fd1baa3e3380f971f86bf1f7747043cf0c3c7459cee2eb4",
    },
    Pin {
        id: 0x0B450017,
        size: 12993,
        sha256: "99762442c38144d8ae82c83b24265c4b1afe3050aab769dc7ae2caddeb12b3e5",
    },
];

pub const FIXED: [[Pin<'static>; 3]; 4] = [
    [
        // ja
        Pin {
            id: 0x0B4508F0,
            size: 34749,
            sha256: "a518d206435024618e64d93b8f5595c9f1236ce1108c8085af0560a5b40e94da",
        },
        Pin {
            id: 0x0B4508F1,
            size: 360,
            sha256: "bae12ede849c94a4203483c7b85379914fa0226236bfd04315fccea3685495a3",
        },
        Pin {
            id: 0x0B4508F2,
            size: 11704,
            sha256: "4bc9348e2488b84711f19b0f8bd4e3845565663d88a8f31c71d2ae78a57bbe93",
        },
    ],
    [
        // en
        Pin {
            id: 0x0B4508EA,
            size: 22449,
            sha256: "255216f19cb34331f0ed774d62cab53469eaefbcb04d7e689bfe6392937684aa",
        },
        Pin {
            id: 0x0B4508EB,
            size: 360,
            sha256: "bae12ede849c94a4203483c7b85379914fa0226236bfd04315fccea3685495a3",
        },
        Pin {
            id: 0x0B4508EC,
            size: 11704,
            sha256: "4a41467264a52f1a13ca4d5e80a21fc89f659dc3fb55a033d9979baa9894b796",
        },
    ],
    [
        // de
        Pin {
            id: 0x0B4508E7,
            size: 24188,
            sha256: "5bc177fd53769790857766bb085ae97bee2161b603e7961549cb3e815df1ff5d",
        },
        Pin {
            id: 0x0B4508E8,
            size: 360,
            sha256: "bae12ede849c94a4203483c7b85379914fa0226236bfd04315fccea3685495a3",
        },
        Pin {
            id: 0x0B4508E9,
            size: 11704,
            sha256: "4dd0c11a55bb2bebf3ffa1b72f4791b9ae087341b22e522d48fde72340590a6e",
        },
    ],
    [
        // fr
        Pin {
            id: 0x0B4508ED,
            size: 27259,
            sha256: "1c24778f39d9aec27e7899f679d40a442ad6c6734df27aab1f08b5883e2a1f46",
        },
        Pin {
            id: 0x0B4508EE,
            size: 360,
            sha256: "bae12ede849c94a4203483c7b85379914fa0226236bfd04315fccea3685495a3",
        },
        Pin {
            id: 0x0B4508EF,
            size: 11704,
            sha256: "6e37938e30c325e745dfe13b8047925493a36d607a1e47f8e2d1120260bde374",
        },
    ],
];
pub const PLACE: [[Pin<'static>; 3]; 4] = [
    [
        // ja
        Pin {
            id: 0x0B450886,
            size: 22572,
            sha256: "402613cf3a2f8888871e309ba9835d666733a259ebefd803bb2f4e111f1c7bef",
        },
        Pin {
            id: 0x0B450887,
            size: 240,
            sha256: "39f7f9dcfbf0a8c1dc23298a814ed822fa0ff8ec58721f8c3fec74d0ff22150b",
        },
        Pin {
            id: 0x0B450888,
            size: 32008,
            sha256: "2b6b945378b9f9eadcffdd0ed807ec5f2e2135f42badfd9102b166f8f0fd73e1",
        },
    ],
    [
        // en
        Pin {
            id: 0x0B450880,
            size: 36300,
            sha256: "fff2d6a92f1a8ee2927c9cf07d31fba5591d8701b31d0dc80a0cda18681ea4df",
        },
        Pin {
            id: 0x0B450881,
            size: 240,
            sha256: "39f7f9dcfbf0a8c1dc23298a814ed822fa0ff8ec58721f8c3fec74d0ff22150b",
        },
        Pin {
            id: 0x0B450882,
            size: 32008,
            sha256: "5581268cc892ea34c55621bd9491542595c4071bdd914cd2a9dcd5191cbac40d",
        },
    ],
    [
        // de
        Pin {
            id: 0x0B45087D,
            size: 67532,
            sha256: "de66224a8432a89cae7c2bcf65e7af4124c61e10a37f0b9f3de4b261dab99267",
        },
        Pin {
            id: 0x0B45087E,
            size: 240,
            sha256: "39f7f9dcfbf0a8c1dc23298a814ed822fa0ff8ec58721f8c3fec74d0ff22150b",
        },
        Pin {
            id: 0x0B45087F,
            size: 32008,
            sha256: "910a32740812933c99c66607acd39c94d7417b591f06a0bbc45d79498432ce39",
        },
    ],
    [
        // fr
        Pin {
            id: 0x0B450883,
            size: 96395,
            sha256: "4cc09c3b906c264adc635a06c88d72b1fad4ef9d47e1f0402a081808231237d1",
        },
        Pin {
            id: 0x0B450884,
            size: 240,
            sha256: "39f7f9dcfbf0a8c1dc23298a814ed822fa0ff8ec58721f8c3fec74d0ff22150b",
        },
        Pin {
            id: 0x0B450885,
            size: 32008,
            sha256: "825676b9738f72d08aa344c5fd829954039981b45ffd9c7cdedfa1aa03c854ea",
        },
    ],
];
