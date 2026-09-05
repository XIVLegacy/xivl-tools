# Command profile source identities

These source identities support the exact-path lookup and selected getter
facts in [Command level-adjustment profiles](command-formula-profiles.md).
They cover the 74 mapped command scripts and nine additional ancestor
scripts from extraction `2012.09.19.0001`.

For each class path below, the source is
`xivl-client-scripts:lua/scripts/<lowercase-class-path-without-leading-slash>.lua`.
SHA-256 identifies that exact script, as retained in
`xivl-client-scripts:manifests/scripts.json`. Parent paths come from the
matching `require` and `_defineClass` / `_defineBaseClass` declaration at
the start of each script. `CommandBaseClass` is the native binding root;
its script attaches methods without declaring another Lua parent.

Inheritance follows these exact paths, not a directory prefix or a global
class-name match. In particular, the mapped
`/Command/Game/Prog/EquipPartsShowHideCommand` derives directly from
`GameCommandBaseClass`. The separately located script with the same leaf
class name is not substituted for it.

| Class path | Declared parent path | Source SHA-256 |
|---|---|---|
| `/Command/AutoAttackTargetChangeCommand` | `/Command/CommandBaseClass` | `2d024196a55fd37c09c5262021ffb1f87dc9087b1ac5b3d1bb4d878d02a01a98` |
| `/Command/ChangeJobCommand` | `/Command/Game/GameCommandBaseClass` | `bb69d42e966da13cbb358778dd638b34df44ea6eac77c6083f9d30fb28a7299f` |
| `/Command/CommandBaseClass` | Native binding root | `bf7dd5fa7a6530c0c3b683be16a252b7ffd62492bdc268f36322977dee4d3e31` |
| `/Command/DebugInputCommand` | `/Command/CommandBaseClass` | `64a6338c60cb62844e2d58efdddb609f880e3e7aed7edcd29709fde46131462c` |
| `/Command/EquipAbilityCommand` | `/Command/Game/GameCommandBaseClass` | `e30ce52a98f6da6d1dae862d27cafb5e34ece9c8ece5f4786707779c12d4f383` |
| `/Command/EquipCommand` | `/Command/Game/GameCommandBaseClass` | `6984eac97e7ded09457f12654a1ff6cdf566c76f218b2bbe120ac47ccb0d970b` |
| `/Command/Game/Ability/Ability` | `/Command/Game/Ability/AbilityBaseClass` | `3d62864ea275d7753aa50fe781b86648cbcd3204d480474dab9bd23e2fe18379` |
| `/Command/Game/Ability/AbilityBaseClass` | `/Command/Game/BattleCommandBaseClass` | `2278cde95818f9a29a692d3a4fdecf947b40b7ad5ebf3f9348067b4dfc1d8674` |
| `/Command/Game/Ability/AttackAbility` | `/Command/Game/Ability/AbilityBaseClass` | `ac63db2594318400ebc8ef376602bace883d4e0dc223367db4280b67d5bb4b87` |
| `/Command/Game/Ability/CmnAbility` | `/Command/Game/Ability/AbilityBaseClass` | `f91a2994361aca4cbbc5e3923b3a50cace75a41efe4e7784d7759231bb8c08aa` |
| `/Command/Game/Ability/CmnCrafterAbility` | `/Command/Game/Ability/AbilityBaseClass` | `96033e48e4837b7cdd3f347fd7e6d3efc8247d7e805dfadd03b34de4e2139062` |
| `/Command/Game/Ability/GathererStealthAbility` | `/Command/Game/Ability/AbilityBaseClass` | `ef454b51d9e5e74c9649925ca67819686b03b74edb196a1128a8c845beb9bb93` |
| `/Command/Game/Ability/MonsterAbility` | `/Command/Game/Ability/AbilityBaseClass` | `95797130d4697bfc2f95f52604acca12e509717c43d711e87fbcc9b39f7e5c56` |
| `/Command/Game/Ability/MonsterSubStatAbility` | `/Command/Game/Ability/AbilityBaseClass` | `e13abf934486a7a5d75620bc3f3a0e75e758c91152ab9c176e8c6a71ec763c77` |
| `/Command/Game/Ability/PointSearchAbility` | `/Command/Game/Ability/AbilityBaseClass` | `8246168af9e9b11ed0fd2545c26f91205ef21994bb9fee5c66ea7236dab9c711` |
| `/Command/Game/AcnItemCreateCommand` | `/Command/Game/GameCommandBaseClass` | `2a81e8659791f185e43e242b691a38e07210babe263e9526737fd96187fc8fc8` |
| `/Command/Game/AcnItemPutCommand` | `/Command/Game/GameCommandBaseClass` | `4b6b0c3922884ccc88358d7945a2ab8bf8155b3b6b2cebf5f5a950d75d55c6da` |
| `/Command/Game/ActivateCommand` | `/Command/Game/GameCommandBaseClass` | `389e5fa09cc272ab6b0c90f2621cb1621ec3f152b68ea19914c06c7326d06c76` |
| `/Command/Game/ArrowReloadCommand` | `/Command/Game/BattleCommandBaseClass` | `158dbc0cbaf5c13122334129985b03193e366dbbe19fe963880eec513acc23d5` |
| `/Command/Game/ArrowStockCommand` | `/Command/Game/BattleCommandBaseClass` | `a22c41b77e316dfed9ea5842dc2205ef4af9ac004fabee7642ca9a0ec78f6006` |
| `/Command/Game/AttackCommand` | `/Command/Game/BattleCommandBaseClass` | `3ea5f2f9749399019a21172925e82140a3eaab3ceaf950472aedc507bb86a7a6` |
| `/Command/Game/Basic/GarudaOthers` | `/Command/Game/BattleCommandBaseClass` | `46c417c9bea4e8d7bc234380b02cc317069fc8a88678cabf3eddd1b6d3122fe1` |
| `/Command/Game/Basic/MonsterAttackCommand` | `/Command/Game/BattleCommandBaseClass` | `5210ce17e911347c49ab9e32417be25ba64aa73d4a1cdb66bc881e3eb731460c` |
| `/Command/Game/Basic/MonsterOthers` | `/Command/Game/BattleCommandBaseClass` | `b45dfe3db051bcdcaef707e26ef8a59739510a2e428c6cc8f57863189bcd9e67` |
| `/Command/Game/Basic/MonsterRangeAttack` | `/Command/Game/BattleCommandBaseClass` | `5a279948d4ac58bb67273db3f107071dc0dda80ce2119b2899d5a4954dfa690e` |
| `/Command/Game/Basic/MonsterShieldCommand` | `/Command/Game/BattleCommandBaseClass` | `b2111383357f809e6fafc3b65ef942106d1f5f4d335c5a487c1158a6e99bf3d1` |
| `/Command/Game/Basic/MonsterSubStatOthers` | `/Command/Game/BattleCommandBaseClass` | `11f50240988c54ef7aea2f78324796cd04e060db98d3f80461c3e417574589de` |
| `/Command/Game/BattleCommandBaseClass` | `/Command/Game/GameCommandBaseClass` | `0eb0b8c77b05128461d94ca1a9bee9b65bccf397ab8efd60903c448915d1e757` |
| `/Command/Game/BewareCommand` | `/Command/Game/GameCommandBaseClass` | `8ee2b25041ef1e9f17f051cda1e22977b011a629173812ffe6977e630eafc751` |
| `/Command/Game/BonusPointCommand` | `/Command/System/SystemCommandBaseClass` | `1d5291268d165302fa646e5bd85283f550e90e9036d20667566bf15348b30a0b` |
| `/Command/Game/BoostPointCommand` | `/Command/Game/GameCommandBaseClass` | `f3eb2b626e480e7c154c85b500245704f9aae7261b2bedcbff1cc594f0d11cbb` |
| `/Command/Game/ChangeEquipSetCommand` | `/Command/Game/GameCommandBaseClass` | `53568ae20767c4b0c835018ccc87e0bc67b889a5ce64b33e53c67e15a616a651` |
| `/Command/Game/CombinationManagementCommand` | `/Command/Game/GameCommandBaseClass` | `5a7c04747cc14760188a910add2c221d2dcabe998d61cdf78e53ff42f3ce8110` |
| `/Command/Game/CombinationStartCommand` | `/Command/Game/GameCommandBaseClass` | `bcc82286234be22d766bba6c85c7c21cd384c251274028a64f2a3bb21d7b9dc9` |
| `/Command/Game/CommandCancelCommand` | `/Command/Game/GameCommandBaseClass` | `0dc50dbfe34484ce2787761b0c5d1b80772a0b742c2ab71d5db7cf1602d3655c` |
| `/Command/Game/Constance/CmnConstance` | `/Command/Game/Constance/ConstanceBaseClass` | `e45fb5fd893882f54e0d3c4fadcc9fa0ba20391f5f4773f56871c0d13852292a` |
| `/Command/Game/Constance/ConstanceBaseClass` | `/Command/Game/BattleCommandBaseClass` | `91b8190c11273eb5443be4daf0479ca1c450c2b4b64cdd73f98f4c4ac898a130` |
| `/Command/Game/CraftCommand` | `/Command/Game/GameCommandBaseClass` | `7f4e8f7d1cc81432753f6798b3e26050ea6642995980dfbafd1e849f325578fa` |
| `/Command/Game/DummyCommand` | `/Command/Game/GameCommandBaseClass` | `b996d29d09e84fce96e921c0af8add639e663595c9ca36ab891c6457019684f0` |
| `/Command/Game/GameCommandBaseClass` | `/Command/CommandBaseClass` | `75f366ca597f77a8e4b506fa8d7b214171cfdbb8d913fa12aa685d72a0b3256b` |
| `/Command/Game/HealingCommand` | `/Command/Game/GameCommandBaseClass` | `9e030d852f34f119c49e41a1822aa3aa94e2159454765d613bef473973ea2c6d` |
| `/Command/Game/HighsenseCommand` | `/Command/Game/GameCommandBaseClass` | `c788db3249c84f0a91d3908659fa7b746b854de00c90885e83af6dd667be10a6` |
| `/Command/Game/Magic/AncientMagic` | `/Command/Game/Magic/MagicBaseClass` | `e4c70004ce026a977b985bf16e394a5b5f0a60f1e3db21bac44d0edc00acba2c` |
| `/Command/Game/Magic/AttackMagic` | `/Command/Game/Magic/MagicBaseClass` | `b1b5f759a55b1475f6488defdfd56cad7d4c2e8137aa0e86bd4ef80a9d5faa90` |
| `/Command/Game/Magic/CmnAbsorptionMagic` | `/Command/Game/Magic/MagicBaseClass` | `6222d4f4872cbd4a3806ad5e161f13392a55126d2bfd154ab33ce9847269b07f` |
| `/Command/Game/Magic/CmnAttackMagic` | `/Command/Game/Magic/MagicBaseClass` | `50c661bd35dcd77316108eeef8d63f85cb27800ed27774649e9b5923e3adc30d` |
| `/Command/Game/Magic/CmnBadStatusMagic` | `/Command/Game/Magic/MagicBaseClass` | `2106461769b339c9d0f2636fce4e8fee13383aec196f708a47aabd570e985332` |
| `/Command/Game/Magic/CmnCureMagic` | `/Command/Game/Magic/MagicBaseClass` | `f4bb71c9807b11521e9435e80b78f7951603e56328f4cdb1811cd61243ed1f47` |
| `/Command/Game/Magic/CmnDrainMagic` | `/Command/Game/Magic/MagicBaseClass` | `b4388a656dd9e24efa9a85d60bdb453abcb1c02276f59ae987a78d8f2c1f3232` |
| `/Command/Game/Magic/CmnGoodStatusMagic` | `/Command/Game/Magic/MagicBaseClass` | `dc73368b2937663a7456183cc2a244bd32b4f1c090ddd00429a1c900f7c2c1d5` |
| `/Command/Game/Magic/CmnRemoveStatusMagic` | `/Command/Game/Magic/MagicBaseClass` | `bfb2936ef84e05dede067b6111e30e4c26d7aa5c327ee2370b54700d83eccb54` |
| `/Command/Game/Magic/CureMagic` | `/Command/Game/Magic/MagicBaseClass` | `762b5d47ca85964d5be86120379d4d19f8ea75cb75579f5d8f75d644968e511c` |
| `/Command/Game/Magic/CuregaMagic` | `/Command/Game/Magic/MagicBaseClass` | `393e80f168e9db4a030474ab21e8c640af666c8c208f5e3d486346990e9f1dfb` |
| `/Command/Game/Magic/EffectMagic` | `/Command/Game/Magic/MagicBaseClass` | `1fe5cb73d14b1ff0b350c9be8cb5eb87671f07401ad543545f7b33425cbf43f5` |
| `/Command/Game/Magic/EsunaMagic` | `/Command/Game/Magic/MagicBaseClass` | `d0e28d72b3513cf05d718e670b69f7b74505cd17a20e59706091ef7fa1cac710` |
| `/Command/Game/Magic/MagicBaseClass` | `/Command/Game/BattleCommandBaseClass` | `83729c1db192e8ef524e5d773d92e814c27b1ccd51be3640d7e57125f3d3c90f` |
| `/Command/Game/Magic/RaiseMagic` | `/Command/Game/Magic/MagicBaseClass` | `b357907b3880309b826f0042b1922d54ad24ae0977ab619a24f5912689a97fe3` |
| `/Command/Game/Magic/SongMagic` | `/Command/Game/Magic/MagicBaseClass` | `d56b8b3a23d53f61d9a83e0bf9211f5f509848626a5ed399100cf53277947690` |
| `/Command/Game/NegotiationCommand` | `/Command/Game/GameCommandBaseClass` | `6aaad74fae6e1174c1fca77e6a2ef9291cbf83406f72e656c9a6434b9ed1d23b` |
| `/Command/Game/PartyTargetCommand` | `/Command/Game/GameCommandBaseClass` | `9cc7e8ba165e6dac4cb3b3db62f48588252b5d67c203bf5a0a9efe61bb4a8b61` |
| `/Command/Game/Prog/ChocoboRideCommand` | `/Command/Game/Prog/ProgCommandBaseClass` | `64213d235d57b7437e998bc090f4c9270dd797910ac43f7a5a01c1a4afac5c03` |
| `/Command/Game/Prog/EquipPartsShowHideCommand` | `/Command/Game/GameCommandBaseClass` | `0d5d18935b93e524b40f7e0d88695fd402f990b3bda1b3c3227efe3da8e3b103` |
| `/Command/Game/Prog/ProgCommandBaseClass` | `/Command/Game/GameCommandBaseClass` | `da4c70fc62ea6bf52d61bb2b3eeea898d663b0f42ae951ebf0ebd0eca538249f` |
| `/Command/Game/ResetOccupiedCommand` | `/Command/Game/GameCommandBaseClass` | `1ccdf1ea85f3566de96e8b0f4c234b95a40cdd4eb2f03ed90390930ea71ed3e2` |
| `/Command/Game/ShieldDefenceCommand` | `/Command/Game/BattleCommandBaseClass` | `fe78443ba7fd83159feb35967b8413e1e15fc124493413dd3b6dc7d9ffb88acf` |
| `/Command/Game/ShieldEffectCommand` | `/Command/Game/GameCommandBaseClass` | `a12a9cd0064cd2049275e9ae1e62e588cac045b6f9dd7685b7ac7564b606b8e0` |
| `/Command/Game/ShotCommand` | `/Command/Game/BattleCommandBaseClass` | `b045d619889eb4a4ba7882abae1c29a2b7a259b3522795ff7e9efba8ac24ba95` |
| `/Command/Game/ThrowCommand` | `/Command/Game/BattleCommandBaseClass` | `4c13a377bd3342776cc39ac50f2aa03769a15badee8906d7df2af1c435672f6c` |
| `/Command/Game/WeaponSkill/AttackWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `76b7336f7030af8df3c20532d96bf46d4f76344c37582c5b5806439b1f044092` |
| `/Command/Game/WeaponSkill/CmnAttackWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `bf88487669c5f22ede007aa247f1ddeebbe87d5c7eda54e14f45187b8e155de9` |
| `/Command/Game/WeaponSkill/DevideAttackWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `ef47774b75f75d3e4703af337a599667b0d9912b17ad210cc110f6b113716755` |
| `/Command/Game/WeaponSkill/GarudaAttackWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `a80ee8b2cd00a3e56909a09d57485d092ae7f7ee20d1be05638f08d7db98116a` |
| `/Command/Game/WeaponSkill/IfritAttackWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `fe17d7077c9820261d656b6cae36362d695b11b6365836c1b636dc8b90f8b391` |
| `/Command/Game/WeaponSkill/IfritSubStatWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `2e26eb110fda5bc60677a39eb60c224928b0c6c920eef13ccf8c3802cbda72d8` |
| `/Command/Game/WeaponSkill/MonsterAbsorbWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `c90e1238be8b9a757414f1d08a70e33f158b2599bc14ed714a81843e1f210aee` |
| `/Command/Game/WeaponSkill/MonsterAttackWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `d5b8e884aad2ca2cfe5cfa96cf5e029d975a32bb0bc1742873ded2f3a78b668e` |
| `/Command/Game/WeaponSkill/MonsterSubStatWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `d66d3e511faac288b5e182d0daa6ae02dffb820d420119de41b6b9c84a311495` |
| `/Command/Game/WeaponSkill/MonsterTest` | `/Command/Game/GameCommandBaseClass` | `f093a963d007e09d79e522bb8c0d29d873aaa0ef382e352b1a5f9778f0435e94` |
| `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `/Command/Game/BattleCommandBaseClass` | `3b81ee1cd014cf1162d9c55e2d441776e804691660eaa32faa96a621af302efa` |
| `/Command/Game/WeaponSkill/WhiteGeneralAttackWeaponSkill` | `/Command/Game/WeaponSkill/WeaponSkillBaseClass` | `94578ef915665dfb38fdaa96216a73fe42004f0e42b891bf582f7fcd4d543060` |
| `/Command/ItemCommand` | `/Command/CommandBaseClass` | `3302b9e1d0f1fb763542233666617d04c6f75c468bd4f49df33c69526b0706f6` |
| `/Command/System/ReserveInputOperationCommand` | `/Command/Game/GameCommandBaseClass` | `7346ad0ad59138bd8bdce615b77eeef6fefbb1903083018ca1467714f5d26de5` |
| `/Command/System/SystemCommandBaseClass` | `/Command/CommandBaseClass` | `4e71e311c9a06dfbef1ea56f9276414099844db7a0dc185557da561bce3b6efe` |

## Selected getter definitions

The selected scope is the distance-limit pair and the eight high/low
parameter-blend getters. These are the only definitions in the pinned
command-script ancestry. Each listed body returns constants. All other
scripts above were inspected in full for selected getter overrides.

| Source class | Getter definitions, source lines |
|---|---|
| `AttackCommand` | `getCommandLevelAdjustLevelMax`: 359 |
| `MonsterAttackCommand` | `getCommandLevelAdjustLevelMax`: 200 |
| `GameCommandBaseClass` | `getCommandLevelAdjustLevelMax`: 1365; `getCommandParam1AdjustForHighLevelUse`: 1436; `getCommandParam2AdjustForHighLevelUse`: 1445; `getCommandParam3AdjustForHighLevelUse`: 1454; `getCommandParam4AdjustForHighLevelUse`: 1463; `getCommandParam1AdjustForLowLevelUse`: 1472; `getCommandParam2AdjustForLowLevelUse`: 1481; `getCommandParam3AdjustForLowLevelUse`: 1490; `getCommandParam4AdjustForLowLevelUse`: 1499 |
| `AncientMagic` | `getCommandLevelAdjustLevelMax`: 11; `getCommandParam1AdjustForHighLevelUse`: 21; `getCommandParam2AdjustForHighLevelUse`: 30; `getCommandParam3AdjustForHighLevelUse`: 39; `getCommandParam4AdjustForHighLevelUse`: 48; `getCommandParam1AdjustForLowLevelUse`: 57; `getCommandParam2AdjustForLowLevelUse`: 66; `getCommandParam3AdjustForLowLevelUse`: 75; `getCommandParam4AdjustForLowLevelUse`: 84 |
| `CmnAttackMagic` | `getCommandLevelAdjustLevelMax`: 11; `getCommandParam1AdjustForHighLevelUse`: 21; `getCommandParam2AdjustForHighLevelUse`: 30; `getCommandParam3AdjustForHighLevelUse`: 39 |
| `CmnBadStatusMagic` | `getCommandParam1AdjustForHighLevelUse`: 11; `getCommandParam2AdjustForHighLevelUse`: 20; `getCommandParam3AdjustForHighLevelUse`: 29 |
| `CmnCureMagic` | `getCommandParam1AdjustForHighLevelUse`: 11; `getCommandParam2AdjustForHighLevelUse`: 20; `getCommandParam3AdjustForHighLevelUse`: 29 |
| `CmnDrainMagic` | `getCommandLevelAdjustLevelMax`: 63; `getCommandParam1AdjustForHighLevelUse`: 73; `getCommandParam2AdjustForHighLevelUse`: 82; `getCommandParam3AdjustForHighLevelUse`: 91 |
| `CmnGoodStatusMagic` | `getCommandParam1AdjustForHighLevelUse`: 11; `getCommandParam2AdjustForHighLevelUse`: 20; `getCommandParam3AdjustForHighLevelUse`: 29 |
| `ShotCommand` | `getCommandLevelAdjustLevelMax`: 38 |
| `ThrowCommand` | `getCommandLevelAdjustLevelMax`: 46 |
